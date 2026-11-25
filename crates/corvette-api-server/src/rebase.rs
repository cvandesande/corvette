//! Serves stored pieces on one continuous timeline: hand-written box walks over bytes in memory that read a piece's init prefix and fragments and shift fragment decode times. No I/O; stored bytes are never touched, and every rewrite happens on an owned copy.

use std::fmt;

const BOX_HEADER_LEN: usize = 8;
const BOX_TO_FILE_END: u32 = 0;
const BOX_LARGE_SIZE: u32 = 1;
const LARGE_SIZE_HEADER_LEN: usize = 16;
const TFHD_BASE_DATA_OFFSET: u32 = 0x0000_0001;
const TFHD_SAMPLE_DESCRIPTION_INDEX: u32 = 0x0000_0002;
const TFHD_DEFAULT_DURATION: u32 = 0x0000_0008;
const TFHD_DEFAULT_SAMPLE_SIZE: u32 = 0x0000_0010;
const TFHD_DEFAULT_SAMPLE_FLAGS: u32 = 0x0000_0020;
const TFHD_KNOWN_FLAGS: u32 = 0x0003_003B;
const TFDT_V1_BODY_LEN: usize = 12;
const EXTENDED_TIME_VERSION: u8 = 1;
const TRUN_DATA_OFFSET: u32 = 0x0000_0001;
const TRUN_FIRST_SAMPLE_FLAGS: u32 = 0x0000_0004;
const TRUN_SAMPLE_DURATION: u32 = 0x0000_0100;
const TRUN_SAMPLE_SIZE: u32 = 0x0000_0200;
const TRUN_SAMPLE_FLAGS: u32 = 0x0000_0400;
const STSD_TABLE_OFFSET: usize = 8;
const SAMPLE_ENTRY_COMMON_LEN: usize = 78;
const SAMPLE_ENTRY_WIDTH_OFFSET: usize = 24;
const SAMPLE_ENTRY_HEIGHT_OFFSET: usize = 26;
const MDHD_V1_TIMESCALE_OFFSET: usize = 20;
const MDHD_V0_TIMESCALE_OFFSET: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseError {
    Truncated,
    Malformed,
    Overflow,
}

impl fmt::Display for RebaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "a box or field runs past the end of the buffer"),
            Self::Malformed => write!(f, "a box declares bytes the walk cannot read"),
            Self::Overflow => write!(f, "a decode-time sum exceeds the 64-bit field"),
        }
    }
}

impl std::error::Error for RebaseError {}

#[derive(Clone, Copy, Debug)]
struct BoxRef {
    kind: [u8; 4],
    start: usize,
    body_start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug)]
struct MoofTimes {
    duration_ticks: u64,
    base_decode_ticks: u64,
    tfdt_field_start: usize,
}

#[derive(Clone, Debug)]
pub struct PieceParts {
    pub init_len: usize,
    pub fragments: Vec<FragmentPart>,
}

#[derive(Clone, Copy, Debug)]
pub struct FragmentPart {
    pub start: usize,
    pub end: usize,
    pub duration_ticks: u64,
    pub base_decode_ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitFingerprint {
    pub sample_entry_fourcc: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub codec_config: Vec<u8>,
    pub timescale: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct TimelinePiece {
    pub run_index: usize,
    pub offset_ticks: u64,
    pub has_discontinuity_before: bool,
}

/// The serve-time plan for a piece sequence: per-piece placement plus, per run, the index of its first piece. After every discontinuity a piece's `offset_ticks` restarts at 0, so it is relative to the start of its own discontinuity sequence and the playlist marker carries the time jump.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pieces: Vec<TimelinePiece>,
    run_first_pieces: Vec<usize>,
    sequence: Option<SequenceState>,
}

#[derive(Clone, Debug)]
struct SequenceState {
    fingerprint: InitFingerprint,
    next_offset: u64,
}

/// Reads a piece's bytes into an init prefix plus one `FragmentPart` per `moof`/`mdat` pair.
///
/// # Errors
///
/// `Truncated` when a box runs past the buffer; `Malformed` when the bytes are not an init prefix with a `moov` followed by complete `moof`/`mdat` pairs.
pub fn split_piece(bytes: &[u8]) -> Result<PieceParts, RebaseError> {
    let boxes = top_level_boxes(bytes)?;
    let first_moof = boxes
        .iter()
        .position(|b| b.kind == *b"moof")
        .ok_or(RebaseError::Malformed)?;
    let init_prefix = &boxes[..first_moof];
    if !init_prefix.iter().any(|b| b.kind == *b"moov")
        || init_prefix.iter().any(|b| b.kind == *b"mdat")
    {
        return Err(RebaseError::Malformed);
    }
    let mut fragments = Vec::new();
    let mut cursor = first_moof;
    while cursor < boxes.len() {
        if boxes[cursor].kind != *b"moof" {
            return Err(RebaseError::Malformed);
        }
        let mdat = boxes.get(cursor + 1).ok_or(RebaseError::Malformed)?;
        if mdat.kind != *b"mdat" {
            return Err(RebaseError::Malformed);
        }
        let times = moof_times(bytes, boxes[cursor])?;
        fragments.push(FragmentPart {
            start: boxes[cursor].start,
            end: mdat.end,
            duration_ticks: times.duration_ticks,
            base_decode_ticks: times.base_decode_ticks,
        });
        cursor += 2;
    }
    Ok(PieceParts {
        init_len: boxes[first_moof].start,
        fragments,
    })
}

/// Returns the stream parameters an init commits a decoder to, so equal-parameter inits compare equal.
///
/// # Errors
///
/// `Malformed` when the `moov/trak/mdia/minf/stbl/stsd` walk or the `mdhd` timescale cannot be read; `Truncated` when the buffer ends inside a box.
pub fn fingerprint(init: &[u8]) -> Result<InitFingerprint, RebaseError> {
    let top = top_level_boxes(init)?;
    let moov = first_box(&top, *b"moov").ok_or(RebaseError::Malformed)?;
    let trak = only_child(init, moov, *b"trak")?.ok_or(RebaseError::Malformed)?;
    let mdia = only_child(init, trak, *b"mdia")?.ok_or(RebaseError::Malformed)?;
    let mdhd = only_child(init, mdia, *b"mdhd")?.ok_or(RebaseError::Malformed)?;
    let timescale = mdhd_timescale(init, mdhd)?;
    let minf = only_child(init, mdia, *b"minf")?.ok_or(RebaseError::Malformed)?;
    let stbl = only_child(init, minf, *b"stbl")?.ok_or(RebaseError::Malformed)?;
    let stsd = only_child(init, stbl, *b"stsd")?.ok_or(RebaseError::Malformed)?;
    let entry = first_sample_entry(init, stsd)?;
    let header = entry
        .start
        .checked_add(BOX_HEADER_LEN)
        .ok_or(RebaseError::Malformed)?;
    let width = read_u16(init, header + SAMPLE_ENTRY_WIDTH_OFFSET).ok_or(RebaseError::Malformed)?;
    let height =
        read_u16(init, header + SAMPLE_ENTRY_HEIGHT_OFFSET).ok_or(RebaseError::Malformed)?;
    let codec_config = codec_config_bytes(init, entry)?;
    Ok(InitFingerprint {
        sample_entry_fourcc: entry.kind,
        width,
        height,
        codec_config,
        timescale,
    })
}

/// Returns one `moof`/`mdat` pair with the `tfdt` base media decode time increased by `offset_ticks`.
///
/// Every other byte is unchanged, so the fragment keeps its size and its internal byte offsets. The `tfdt` must be version 1: a version-0 `tfdt` is refused as `Malformed` rather than widened, because the bridge always writes version 1.
///
/// # Errors
///
/// `Truncated` when a box runs past the buffer; `Malformed` when the bytes are not a `moof`/`mdat` pair with a readable version-1 `tfdt`; `Overflow` when the shifted decode time exceeds 64 bits.
pub fn rebase_fragment(fragment: &[u8], offset_ticks: u64) -> Result<Vec<u8>, RebaseError> {
    let moof = read_box(fragment, 0, fragment.len())?;
    if moof.kind != *b"moof" {
        return Err(RebaseError::Malformed);
    }
    if moof.end >= fragment.len() {
        return Err(RebaseError::Malformed);
    }
    let mdat = read_box(fragment, moof.end, fragment.len())?;
    if mdat.kind != *b"mdat" || mdat.end != fragment.len() {
        return Err(RebaseError::Malformed);
    }
    let times = moof_times(fragment, moof)?;
    let shifted = times
        .base_decode_ticks
        .checked_add(offset_ticks)
        .ok_or(RebaseError::Overflow)?;
    let mut out = fragment.to_vec();
    let field = out
        .get_mut(times.tfdt_field_start..times.tfdt_field_start + 8)
        .ok_or(RebaseError::Malformed)?;
    field.copy_from_slice(&shifted.to_be_bytes());
    Ok(out)
}

impl Timeline {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pieces: Vec::new(),
            run_first_pieces: Vec::new(),
            sequence: None,
        }
    }

    /// Places the next piece: a fingerprint change starts a new run, either kind of discontinuity restarts its offset at 0, and the first piece is never marked.
    ///
    /// # Errors
    ///
    /// `Overflow` when a duration or cumulative offset exceeds 64 bits; the timeline is left unchanged.
    pub fn push(
        &mut self,
        parts: &PieceParts,
        fingerprint: &InitFingerprint,
        gap_before: bool,
    ) -> Result<(), RebaseError> {
        let duration_ticks = piece_duration_ticks(parts)?;
        let (run_index, offset_ticks, has_discontinuity_before) = match self.sequence.as_ref() {
            None => {
                debug_assert!(self.run_first_pieces.is_empty());
                self.run_first_pieces.push(0);
                (0, 0, false)
            }
            Some(sequence) => {
                let run_change = *fingerprint != sequence.fingerprint;
                let has_discontinuity_before = run_change || gap_before;
                let offset_ticks = if has_discontinuity_before {
                    0
                } else {
                    sequence.next_offset
                };
                if run_change {
                    self.run_first_pieces.push(self.pieces.len());
                }
                (
                    self.run_first_pieces.len() - 1,
                    offset_ticks,
                    has_discontinuity_before,
                )
            }
        };
        let next_offset = offset_ticks
            .checked_add(duration_ticks)
            .ok_or(RebaseError::Overflow)?;
        self.pieces.push(TimelinePiece {
            run_index,
            offset_ticks,
            has_discontinuity_before,
        });
        self.sequence = Some(SequenceState {
            fingerprint: fingerprint.clone(),
            next_offset,
        });
        Ok(())
    }

    #[must_use]
    pub fn pieces(&self) -> &[TimelinePiece] {
        &self.pieces
    }

    #[must_use]
    pub fn run_first_pieces(&self) -> &[usize] {
        &self.run_first_pieces
    }
}

fn piece_duration_ticks(parts: &PieceParts) -> Result<u64, RebaseError> {
    let mut sum = 0u64;
    for fragment in &parts.fragments {
        sum = sum
            .checked_add(fragment.duration_ticks)
            .ok_or(RebaseError::Overflow)?;
    }
    Ok(sum)
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let window = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes(window.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let window = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(window.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let window = bytes.get(at..at.checked_add(8)?)?;
    Some(u64::from_be_bytes(window.try_into().ok()?))
}

fn read_box(bytes: &[u8], start: usize, limit: usize) -> Result<BoxRef, RebaseError> {
    let declared_size = read_u32(bytes, start).ok_or(RebaseError::Truncated)?;
    let kind = <[u8; 4]>::try_from(
        bytes
            .get(start + 4..start + BOX_HEADER_LEN)
            .ok_or(RebaseError::Truncated)?,
    )
    .map_err(|_| RebaseError::Truncated)?;
    let (body_start, end) = if declared_size == BOX_TO_FILE_END {
        let header_end = start
            .checked_add(BOX_HEADER_LEN)
            .ok_or(RebaseError::Truncated)?;
        (header_end, limit)
    } else if declared_size == BOX_LARGE_SIZE {
        let large_size = read_u64(bytes, start + 8).ok_or(RebaseError::Truncated)?;
        let size = usize::try_from(large_size).map_err(|_| RebaseError::Truncated)?;
        if size < LARGE_SIZE_HEADER_LEN {
            return Err(RebaseError::Malformed);
        }
        (
            start + LARGE_SIZE_HEADER_LEN,
            checked_end(start, size, bytes, limit)?,
        )
    } else {
        let size = usize::try_from(declared_size).map_err(|_| RebaseError::Malformed)?;
        if size < BOX_HEADER_LEN {
            return Err(RebaseError::Malformed);
        }
        (
            start + BOX_HEADER_LEN,
            checked_end(start, size, bytes, limit)?,
        )
    };
    Ok(BoxRef {
        kind,
        start,
        body_start,
        end,
    })
}

fn checked_end(
    start: usize,
    size: usize,
    bytes: &[u8],
    limit: usize,
) -> Result<usize, RebaseError> {
    let end = start.checked_add(size).ok_or(RebaseError::Truncated)?;
    if end > bytes.len() {
        return Err(RebaseError::Truncated);
    }
    if end > limit {
        return Err(RebaseError::Malformed);
    }
    Ok(end)
}

fn top_level_boxes(bytes: &[u8]) -> Result<Vec<BoxRef>, RebaseError> {
    let mut boxes = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let top = read_box(bytes, cursor, bytes.len())?;
        cursor = top.end;
        boxes.push(top);
    }
    Ok(boxes)
}

fn children(bytes: &[u8], parent: BoxRef) -> Result<Vec<BoxRef>, RebaseError> {
    let mut kids = Vec::new();
    let mut cursor = parent.body_start;
    while cursor < parent.end {
        let child = read_box(bytes, cursor, parent.end)?;
        cursor = child.end;
        kids.push(child);
    }
    Ok(kids)
}

fn only_child(
    bytes: &[u8],
    parent: BoxRef,
    wanted: [u8; 4],
) -> Result<Option<BoxRef>, RebaseError> {
    let mut found = None;
    for child in children(bytes, parent)? {
        if child.kind == wanted {
            if found.is_some() {
                return Err(RebaseError::Malformed);
            }
            found = Some(child);
        }
    }
    Ok(found)
}

fn first_box(boxes: &[BoxRef], wanted: [u8; 4]) -> Option<BoxRef> {
    boxes.iter().copied().find(|b| b.kind == wanted)
}

fn moof_times(bytes: &[u8], moof: BoxRef) -> Result<MoofTimes, RebaseError> {
    let traf = only_child(bytes, moof, *b"traf")?.ok_or(RebaseError::Malformed)?;
    let tfhd = only_child(bytes, traf, *b"tfhd")?;
    let tfdt = only_child(bytes, traf, *b"tfdt")?.ok_or(RebaseError::Malformed)?;
    let trun = only_child(bytes, traf, *b"trun")?.ok_or(RebaseError::Malformed)?;
    let tfdt_field_start = tfdt
        .body_start
        .checked_add(4)
        .ok_or(RebaseError::Malformed)?;
    if tfdt.body_start + TFDT_V1_BODY_LEN > tfdt.end {
        return Err(RebaseError::Malformed);
    }
    if bytes.get(tfdt.body_start).copied() != Some(EXTENDED_TIME_VERSION) {
        return Err(RebaseError::Malformed);
    }
    let base_decode_ticks = read_u64(bytes, tfdt_field_start).ok_or(RebaseError::Malformed)?;
    let default_duration = tfhd_default_duration(bytes, tfhd)?;
    let duration_ticks = trun_duration(bytes, trun, default_duration)?;
    Ok(MoofTimes {
        duration_ticks,
        base_decode_ticks,
        tfdt_field_start,
    })
}

fn tfhd_default_duration(bytes: &[u8], tfhd: Option<BoxRef>) -> Result<Option<u32>, RebaseError> {
    let Some(tfhd) = tfhd else {
        return Ok(None);
    };
    let flags = read_u32(bytes, tfhd.body_start).ok_or(RebaseError::Malformed)?;
    if flags & !TFHD_KNOWN_FLAGS != 0 {
        return Err(RebaseError::Malformed);
    }
    let mut pos = tfhd
        .body_start
        .checked_add(8)
        .ok_or(RebaseError::Malformed)?;
    for (bit, field_len) in [
        (TFHD_BASE_DATA_OFFSET, 8usize),
        (TFHD_SAMPLE_DESCRIPTION_INDEX, 4),
        (TFHD_DEFAULT_DURATION, 4),
        (TFHD_DEFAULT_SAMPLE_SIZE, 4),
        (TFHD_DEFAULT_SAMPLE_FLAGS, 4),
    ] {
        if flags & bit == 0 {
            continue;
        }
        let field_end = pos.checked_add(field_len).ok_or(RebaseError::Malformed)?;
        if tfhd.end < field_end {
            return Err(RebaseError::Malformed);
        }
        if bit == TFHD_DEFAULT_DURATION {
            return Ok(Some(read_u32(bytes, pos).ok_or(RebaseError::Malformed)?));
        }
        pos = field_end;
    }
    Ok(None)
}

fn trun_duration(
    bytes: &[u8],
    trun: BoxRef,
    default_duration: Option<u32>,
) -> Result<u64, RebaseError> {
    let flags = read_u32(bytes, trun.body_start).ok_or(RebaseError::Malformed)?;
    let sample_count = read_u32(bytes, trun.body_start + 4).ok_or(RebaseError::Malformed)?;
    let mut pos = trun.body_start + 8;
    if flags & TRUN_DATA_OFFSET != 0 {
        pos += 4;
    }
    if flags & TRUN_FIRST_SAMPLE_FLAGS != 0 {
        pos += 4;
    }
    let entry_len = usize::from(flags & TRUN_SAMPLE_DURATION != 0) * 4
        + usize::from(flags & TRUN_SAMPLE_SIZE != 0) * 4
        + usize::from(flags & TRUN_SAMPLE_FLAGS != 0) * 4;
    let count = usize::try_from(sample_count).map_err(|_| RebaseError::Malformed)?;
    let table_len = count.checked_mul(entry_len).ok_or(RebaseError::Malformed)?;
    if pos > trun.end || trun.end - pos < table_len {
        return Err(RebaseError::Malformed);
    }
    if flags & TRUN_SAMPLE_DURATION != 0 {
        let mut sum = 0u64;
        let mut at = pos;
        for _ in 0..count {
            let sample = read_u32(bytes, at).ok_or(RebaseError::Malformed)?;
            sum = sum
                .checked_add(u64::from(sample))
                .ok_or(RebaseError::Overflow)?;
            at += entry_len;
        }
        return Ok(sum);
    }
    let default = default_duration.ok_or(RebaseError::Malformed)?;
    u64::from(default)
        .checked_mul(u64::try_from(count).map_err(|_| RebaseError::Malformed)?)
        .ok_or(RebaseError::Malformed)
}

fn mdhd_timescale(bytes: &[u8], mdhd: BoxRef) -> Result<u32, RebaseError> {
    let version = bytes
        .get(mdhd.body_start)
        .copied()
        .ok_or(RebaseError::Malformed)?;
    let offset = match version {
        0 => MDHD_V0_TIMESCALE_OFFSET,
        EXTENDED_TIME_VERSION => MDHD_V1_TIMESCALE_OFFSET,
        _ => return Err(RebaseError::Malformed),
    };
    if mdhd.body_start + offset + 4 > mdhd.end {
        return Err(RebaseError::Malformed);
    }
    read_u32(bytes, mdhd.body_start + offset).ok_or(RebaseError::Malformed)
}

fn first_sample_entry(bytes: &[u8], stsd: BoxRef) -> Result<BoxRef, RebaseError> {
    let entry_count = read_u32(bytes, stsd.body_start + 4).ok_or(RebaseError::Malformed)?;
    if entry_count != 1 {
        return Err(RebaseError::Malformed);
    }
    read_box(bytes, stsd.body_start + STSD_TABLE_OFFSET, stsd.end)
}

fn codec_config_bytes(bytes: &[u8], entry: BoxRef) -> Result<Vec<u8>, RebaseError> {
    let body = entry
        .start
        .checked_add(BOX_HEADER_LEN)
        .ok_or(RebaseError::Malformed)?;
    let cursor_start = body
        .checked_add(SAMPLE_ENTRY_COMMON_LEN)
        .ok_or(RebaseError::Malformed)?;
    if cursor_start >= entry.end {
        return Err(RebaseError::Malformed);
    }
    let mut cursor = cursor_start;
    while cursor < entry.end {
        let child = read_box(bytes, cursor, entry.end)?;
        if child.kind == *b"avcC" || child.kind == *b"hvcC" {
            return bytes
                .get(child.start..child.end)
                .map(<[u8]>::to_vec)
                .ok_or(RebaseError::Malformed);
        }
        cursor = child.end;
    }
    Err(RebaseError::Malformed)
}
