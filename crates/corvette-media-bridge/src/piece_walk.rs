//! Reads one stored piece's own box bytes to recover the facts a missing
//! index row needs: the fragment-summed duration and the codec name.

/// What a piece's bytes declare about themselves.
#[derive(Debug)]
pub struct PieceFacts {
    pub duration_ms: i64,
    pub codec: &'static str,
}

/// Why a piece's bytes did not yield their facts.
#[derive(Debug)]
pub enum WalkError {
    Truncated,
    BoxSizeInvalid,
    MissingFtyp,
    MissingMoov,
    FragmentOrder,
    NoFragments,
    MissingMdhd,
    TimescaleZero,
    MissingTfdt,
    TrunMalformed,
    MissingStsd,
    SpanInvalid,
}

impl std::fmt::Display for WalkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(
                f,
                "the piece ends inside a box header or field: a box runs past the buffer"
            ),
            Self::BoxSizeInvalid => write!(
                f,
                "a box declares a refused size: 0, an extended 64-bit size, or under the 8-byte header"
            ),
            Self::MissingFtyp => write!(f, "the first top-level box is not 'ftyp'"),
            Self::MissingMoov => write!(f, "the box after 'ftyp' is not 'moov'"),
            Self::FragmentOrder => {
                write!(
                    f,
                    "a top-level box after 'moov' breaks the 'moof' 'mdat' pairing"
                )
            }
            Self::NoFragments => write!(f, "the piece carries no 'moof' 'mdat' pair"),
            Self::MissingMdhd => write!(f, "no 'moov/trak/mdia/mdhd' could be read"),
            Self::TimescaleZero => write!(f, "the media timescale is zero"),
            Self::MissingTfdt => write!(f, "a 'moof' carries no readable 'traf/tfdt' base time"),
            Self::TrunMalformed => {
                write!(
                    f,
                    "a 'trun' is malformed or its sample table runs past the box"
                )
            }
            Self::MissingStsd => {
                write!(
                    f,
                    "no 'moov/trak/mdia/minf/stbl/stsd' sample entry could be read"
                )
            }
            Self::SpanInvalid => write!(
                f,
                "the fragment duration span is zero, negative, or does not fit in milliseconds"
            ),
        }
    }
}

impl std::error::Error for WalkError {}

const BOX_HEADER_LEN: usize = 8;

const TRUN_DATA_OFFSET: u32 = 0x0000_0001;
const TRUN_FIRST_SAMPLE_FLAGS: u32 = 0x0000_0004;
const TRUN_SAMPLE_DURATION: u32 = 0x0000_0100;
const TRUN_SAMPLE_SIZE: u32 = 0x0000_0200;
const TRUN_SAMPLE_FLAGS: u32 = 0x0000_0400;

const MILLIS_PER_SECOND: u64 = 1_000;

#[derive(Clone, Copy)]
struct BoxRef {
    kind: [u8; 4],
    start: usize,
    end: usize,
}

struct PieceLayout {
    moov: BoxRef,
    moofs: Vec<BoxRef>,
}

#[derive(Clone, Copy)]
enum Stage {
    Ftyp,
    Moov,
    Moof,
    Mdat,
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let window = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(<[u8; 4]>::try_from(window).ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let window = bytes.get(at..at.checked_add(8)?)?;
    Some(u64::from_be_bytes(<[u8; 8]>::try_from(window).ok()?))
}

fn read_box(bytes: &[u8], start: usize, limit: usize) -> Result<BoxRef, WalkError> {
    let raw_size = read_u32(bytes, start).ok_or(WalkError::Truncated)?;
    if raw_size == 0 || raw_size == 1 {
        return Err(WalkError::BoxSizeInvalid);
    }
    let size = usize::try_from(raw_size).map_err(|_| WalkError::BoxSizeInvalid)?;
    if size < BOX_HEADER_LEN {
        return Err(WalkError::BoxSizeInvalid);
    }
    let end = start.checked_add(size).ok_or(WalkError::BoxSizeInvalid)?;
    if end > limit || end > bytes.len() {
        return Err(WalkError::Truncated);
    }
    let kind: [u8; 4] = bytes
        .get(start + 4..start + 8)
        .and_then(|window| <[u8; 4]>::try_from(window).ok())
        .ok_or(WalkError::Truncated)?;
    Ok(BoxRef { kind, start, end })
}

fn first_child(bytes: &[u8], parent: BoxRef, wanted: [u8; 4]) -> Option<BoxRef> {
    let mut start = parent.start.checked_add(BOX_HEADER_LEN)?;
    while start < parent.end {
        let child = read_box(bytes, start, parent.end).ok()?;
        if child.kind == wanted {
            return Some(child);
        }
        start = child.end;
    }
    None
}

fn piece_layout(bytes: &[u8]) -> Result<PieceLayout, WalkError> {
    let mut stage = Stage::Ftyp;
    let mut moov: Option<BoxRef> = None;
    let mut moofs: Vec<BoxRef> = Vec::new();
    let mut start = 0usize;
    while start < bytes.len() {
        let top = read_box(bytes, start, bytes.len())?;
        stage = match stage {
            Stage::Ftyp if top.kind == *b"ftyp" => Stage::Moov,
            Stage::Moov if top.kind == *b"moov" => {
                moov = Some(top);
                Stage::Moof
            }
            Stage::Moof if top.kind == *b"moof" => {
                moofs.push(top);
                Stage::Mdat
            }
            Stage::Mdat if top.kind == *b"mdat" => Stage::Moof,
            Stage::Ftyp => return Err(WalkError::MissingFtyp),
            Stage::Moov => return Err(WalkError::MissingMoov),
            Stage::Moof | Stage::Mdat => return Err(WalkError::FragmentOrder),
        };
        start = top.end;
    }
    let moov = match (stage, moov) {
        (Stage::Ftyp, _) => return Err(WalkError::MissingFtyp),
        (Stage::Moov, _) => return Err(WalkError::MissingMoov),
        (Stage::Mdat, _) => return Err(WalkError::FragmentOrder),
        (Stage::Moof, Some(moov)) => moov,
        (Stage::Moof, None) => return Err(WalkError::NoFragments),
    };
    Ok(PieceLayout { moov, moofs })
}

fn media_timescale(bytes: &[u8], moov: BoxRef) -> Result<u64, WalkError> {
    let trak = first_child(bytes, moov, *b"trak").ok_or(WalkError::MissingMdhd)?;
    let mdia = first_child(bytes, trak, *b"mdia").ok_or(WalkError::MissingMdhd)?;
    let mdhd = first_child(bytes, mdia, *b"mdhd").ok_or(WalkError::MissingMdhd)?;
    let body = bytes
        .get(mdhd.start..mdhd.end)
        .ok_or(WalkError::MissingMdhd)?;
    let version = *body.get(BOX_HEADER_LEN).ok_or(WalkError::MissingMdhd)?;
    let timescale_field = match version {
        0 => 20,
        1 => 28,
        _ => return Err(WalkError::MissingMdhd),
    };
    let timescale = u64::from(read_u32(body, timescale_field).ok_or(WalkError::MissingMdhd)?);
    if timescale == 0 {
        return Err(WalkError::TimescaleZero);
    }
    Ok(timescale)
}

fn sample_entry_codec(bytes: &[u8], moov: BoxRef) -> Result<&'static str, WalkError> {
    let trak = first_child(bytes, moov, *b"trak").ok_or(WalkError::MissingStsd)?;
    let mdia = first_child(bytes, trak, *b"mdia").ok_or(WalkError::MissingStsd)?;
    let minf = first_child(bytes, mdia, *b"minf").ok_or(WalkError::MissingStsd)?;
    let stbl = first_child(bytes, minf, *b"stbl").ok_or(WalkError::MissingStsd)?;
    let stsd = first_child(bytes, stbl, *b"stsd").ok_or(WalkError::MissingStsd)?;
    let body = bytes
        .get(stsd.start..stsd.end)
        .ok_or(WalkError::MissingStsd)?;
    let kind: [u8; 4] = body
        .get(20..24)
        .and_then(|window| <[u8; 4]>::try_from(window).ok())
        .ok_or(WalkError::MissingStsd)?;
    if kind == *b"avc1" {
        return Ok("h264");
    }
    if kind == *b"hvc1" || kind == *b"hev1" {
        return Ok("h265");
    }
    Ok("unknown")
}

fn fragment_facts(bytes: &[u8], moof: BoxRef) -> Result<(u64, u64), WalkError> {
    let traf = first_child(bytes, moof, *b"traf").ok_or(WalkError::MissingTfdt)?;
    let tfdt = first_child(bytes, traf, *b"tfdt").ok_or(WalkError::MissingTfdt)?;
    let body = bytes
        .get(tfdt.start..tfdt.end)
        .ok_or(WalkError::MissingTfdt)?;
    let version = *body.get(BOX_HEADER_LEN).ok_or(WalkError::MissingTfdt)?;
    let base = match version {
        0 => u64::from(read_u32(body, 12).ok_or(WalkError::MissingTfdt)?),
        1 => read_u64(body, 12).ok_or(WalkError::MissingTfdt)?,
        _ => return Err(WalkError::MissingTfdt),
    };
    Ok((base, trun_duration_sum(bytes, traf)?))
}

fn trun_duration_sum(bytes: &[u8], traf: BoxRef) -> Result<u64, WalkError> {
    let mut total = 0u64;
    let mut start = traf.start + BOX_HEADER_LEN;
    while start < traf.end {
        let child = read_box(bytes, start, traf.end).map_err(|_| WalkError::TrunMalformed)?;
        if child.kind == *b"trun" {
            total = total
                .checked_add(trun_durations(bytes, child)?)
                .ok_or(WalkError::SpanInvalid)?;
        }
        start = child.end;
    }
    Ok(total)
}

fn trun_durations(bytes: &[u8], trun: BoxRef) -> Result<u64, WalkError> {
    let body = trun
        .start
        .checked_add(BOX_HEADER_LEN)
        .ok_or(WalkError::TrunMalformed)?;
    let flags = read_u32(bytes, body).ok_or(WalkError::TrunMalformed)? & 0x00FF_FFFF;
    let sample_count = read_u32(bytes, body + 4).ok_or(WalkError::TrunMalformed)?;
    let mut table = body + 8;
    if flags & TRUN_DATA_OFFSET != 0 {
        table += 4;
    }
    if flags & TRUN_FIRST_SAMPLE_FLAGS != 0 {
        table += 4;
    }
    if flags & TRUN_SAMPLE_DURATION == 0 {
        return Ok(0);
    }
    let mut entry_len = 4;
    if flags & TRUN_SAMPLE_SIZE != 0 {
        entry_len += 4;
    }
    if flags & TRUN_SAMPLE_FLAGS != 0 {
        entry_len += 4;
    }
    let table_len = usize::try_from(sample_count)
        .ok()
        .and_then(|count| count.checked_mul(entry_len))
        .ok_or(WalkError::TrunMalformed)?;
    let table_end = table
        .checked_add(table_len)
        .ok_or(WalkError::TrunMalformed)?;
    if table_end > trun.end {
        return Err(WalkError::TrunMalformed);
    }
    let samples = bytes
        .get(table..table_end)
        .ok_or(WalkError::TrunMalformed)?;
    let mut total = 0u64;
    for entry in samples.chunks_exact(entry_len) {
        let duration: [u8; 4] = entry
            .get(..4)
            .and_then(|window| <[u8; 4]>::try_from(window).ok())
            .ok_or(WalkError::TrunMalformed)?;
        total = total
            .checked_add(u64::from(u32::from_be_bytes(duration)))
            .ok_or(WalkError::SpanInvalid)?;
    }
    Ok(total)
}

/// Reads a stored piece's own boxes and returns its duration and codec.
///
/// # Errors
/// Returns a [`WalkError`] when the bytes are not a complete tile of `ftyp`,
/// `moov`, and at least one `moof` `mdat` pair, or the fragments do not state
/// a positive duration.
pub fn walk(bytes: &[u8]) -> Result<PieceFacts, WalkError> {
    let layout = piece_layout(bytes)?;
    let timescale = media_timescale(bytes, layout.moov)?;
    let codec = sample_entry_codec(bytes, layout.moov)?;
    let first = *layout.moofs.first().ok_or(WalkError::NoFragments)?;
    let last = *layout.moofs.last().ok_or(WalkError::NoFragments)?;
    let (first_base, _) = fragment_facts(bytes, first)?;
    let (last_base, last_durations) = fragment_facts(bytes, last)?;
    let span_ticks = last_base
        .checked_add(last_durations)
        .and_then(|end| end.checked_sub(first_base))
        .ok_or(WalkError::SpanInvalid)?;
    if span_ticks == 0 {
        return Err(WalkError::SpanInvalid);
    }
    let millis = span_ticks
        .checked_mul(MILLIS_PER_SECOND)
        .and_then(|scaled| scaled.checked_div(timescale))
        .ok_or(WalkError::SpanInvalid)?;
    let duration_ms = i64::try_from(millis).map_err(|_| WalkError::SpanInvalid)?;
    Ok(PieceFacts { duration_ms, codec })
}
