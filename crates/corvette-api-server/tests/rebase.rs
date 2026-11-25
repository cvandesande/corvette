//! Builds stored-piece bytes by hand and pins the rebase library's reads: split ranges and durations, fingerprints, decode-time rewrites, and the timeline plan.

use corvette_api_server::rebase::{
    InitFingerprint, RebaseError, Timeline, fingerprint, rebase_fragment, split_piece,
};

/// One stored piece's init parameters as the fixtures write them.
struct InitSpec {
    creation_time: u32,
    mdhd_version: u8,
    timescale: u32,
    fourcc: [u8; 4],
    width: u16,
    height: u16,
    config_kind: [u8; 4],
    config_body: Vec<u8>,
}

/// What one `trun` declares about its samples.
enum TrunKind {
    /// A `trun` whose per-sample entries carry durations, like the bridge writes.
    SampleDurations(Vec<u32>),
    /// A `trun` with only a sample count, so durations come from a `tfhd` default.
    CountOnly(u32),
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn mp4_box(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(
        &mut out,
        u32::try_from(body.len() + 8).expect("fixture box fits u32"),
    );
    out.extend_from_slice(&kind);
    out.extend_from_slice(body);
    out
}

fn largesize_box(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, 1);
    out.extend_from_slice(&kind);
    put_u64(
        &mut out,
        u64::try_from(body.len() + 16).expect("fixture box fits u64"),
    );
    out.extend_from_slice(body);
    out
}

fn to_end_of_file_box(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, 0);
    out.extend_from_slice(&kind);
    out.extend_from_slice(body);
    out
}

fn ftyp_box() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"iso5");
    put_u32(&mut body, 0);
    body.extend_from_slice(b"iso5iso6isommp41");
    mp4_box(*b"ftyp", &body)
}

fn default_init_spec() -> InitSpec {
    InitSpec {
        creation_time: 0,
        mdhd_version: 0,
        timescale: 90_000,
        fourcc: *b"avc1",
        width: 1920,
        height: 1080,
        config_kind: *b"avcC",
        config_body: vec![0x01, 0x42, 0xC0, 0x1F, 0xE1, 0xFF],
    }
}

fn matrix_and_tail(out: &mut Vec<u8>) {
    for value in [0x0001_0000i32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        put_u32(out, value.cast_unsigned());
    }
}

fn mvhd_body(creation_time: u32) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, 0);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, 90_000);
    put_u32(&mut body, 0);
    put_u32(&mut body, 0x0001_0000);
    put_u16(&mut body, 0x0100);
    put_u16(&mut body, 0);
    put_u64(&mut body, 0);
    matrix_and_tail(&mut body);
    for _ in 0..6 {
        put_u32(&mut body, 0);
    }
    put_u32(&mut body, 2);
    body
}

fn tkhd_body(creation_time: u32, width: u16, height: u16) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0);
    body.extend_from_slice(&[0, 0, 0x07]);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, creation_time);
    put_u32(&mut body, 1);
    put_u32(&mut body, 0);
    put_u32(&mut body, 0);
    put_u64(&mut body, 0);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    matrix_and_tail(&mut body);
    put_u32(&mut body, u32::from(width) << 16);
    put_u32(&mut body, u32::from(height) << 16);
    body
}

fn mdhd_body(version: u8, timescale: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(version);
    body.extend_from_slice(&[0, 0, 0]);
    if version == 1 {
        put_u64(&mut body, 0);
        put_u64(&mut body, 0);
        put_u32(&mut body, timescale);
        put_u64(&mut body, 0);
    } else {
        put_u32(&mut body, 0);
        put_u32(&mut body, 0);
        put_u32(&mut body, timescale);
        put_u32(&mut body, 0);
    }
    put_u16(&mut body, 0x55C4);
    put_u16(&mut body, 0);
    body
}

fn hdlr_body() -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, 0);
    put_u32(&mut body, 0);
    body.extend_from_slice(b"vide");
    put_u64(&mut body, 0);
    put_u32(&mut body, 0);
    body.extend_from_slice(b"test\0");
    body
}

fn sample_entry_body(width: u16, height: u16) -> Vec<u8> {
    let mut body = vec![0u8; 6];
    put_u16(&mut body, 1);
    put_u16(&mut body, 0);
    put_u16(&mut body, 0);
    put_u32(&mut body, 0);
    put_u64(&mut body, 0);
    put_u16(&mut body, width);
    put_u16(&mut body, height);
    put_u32(&mut body, 0x0048_0000);
    put_u32(&mut body, 0x0048_0000);
    put_u32(&mut body, 0);
    put_u16(&mut body, 1);
    body.extend_from_slice(&[0u8; 32]);
    put_u16(&mut body, 0x0018);
    put_u16(&mut body, 0xFFFF);
    body
}

fn stsd_body(spec: &InitSpec) -> Vec<u8> {
    let mut entry_body = sample_entry_body(spec.width, spec.height);
    entry_body.extend_from_slice(&mp4_box(spec.config_kind, &spec.config_body));
    let mut body = Vec::new();
    put_u32(&mut body, 0);
    put_u32(&mut body, 1);
    body.extend_from_slice(&mp4_box(spec.fourcc, &entry_body));
    body
}

fn init_bytes(spec: &InitSpec) -> Vec<u8> {
    init_bytes_with_stsd(spec, &stsd_body(spec))
}

fn init_bytes_with_stsd(spec: &InitSpec, stsd_bytes: &[u8]) -> Vec<u8> {
    let stbl = mp4_box(*b"stbl", &mp4_box(*b"stsd", stsd_bytes));
    let mut minf_body = mp4_box(*b"vmhd", &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    minf_body.extend_from_slice(&stbl);
    let minf = mp4_box(*b"minf", &minf_body);
    let mut mdia_body = mp4_box(*b"mdhd", &mdhd_body(spec.mdhd_version, spec.timescale));
    mdia_body.extend_from_slice(&mp4_box(*b"hdlr", &hdlr_body()));
    mdia_body.extend_from_slice(&minf);
    let mdia = mp4_box(*b"mdia", &mdia_body);
    let mut trak_body = mp4_box(
        *b"tkhd",
        &tkhd_body(spec.creation_time, spec.width, spec.height),
    );
    trak_body.extend_from_slice(&mdia);
    let trak = mp4_box(*b"trak", &trak_body);
    let mut moov_body = mp4_box(*b"mvhd", &mvhd_body(spec.creation_time));
    moov_body.extend_from_slice(&trak);
    let mut out = ftyp_box();
    out.extend_from_slice(&mp4_box(*b"moov", &moov_body));
    out
}

fn tfdt_box(version: u8, value: u64) -> Vec<u8> {
    let mut body = Vec::new();
    if version == 1 {
        put_u32(&mut body, 1 << 24);
        put_u64(&mut body, value);
    } else {
        put_u32(&mut body, 0);
        put_u32(
            &mut body,
            u32::try_from(value).expect("fixture v0 time fits u32"),
        );
    }
    mp4_box(*b"tfdt", &body)
}

fn tfhd_box(default_duration: Option<u32>) -> Vec<u8> {
    let mut body = Vec::new();
    let mut flags = 0x0002_0000u32;
    if default_duration.is_some() {
        flags |= 0x0000_0008;
    }
    put_u32(&mut body, flags);
    put_u32(&mut body, 1);
    if let Some(duration) = default_duration {
        put_u32(&mut body, duration);
    }
    mp4_box(*b"tfhd", &body)
}

fn tfhd_with_raw_flags(flags: u32, fields: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(&mut body, flags);
    put_u32(&mut body, 1);
    body.extend_from_slice(fields);
    mp4_box(*b"tfhd", &body)
}

fn fragment_with_tfhd(tfhd: &[u8], trun: TrunKind) -> Vec<u8> {
    let mut mfhd_body = Vec::new();
    put_u32(&mut mfhd_body, 0);
    put_u32(&mut mfhd_body, 1);
    let mut traf_body = tfhd.to_vec();
    traf_body.extend_from_slice(&tfdt_box(1, 0));
    traf_body.extend_from_slice(&trun_box(trun));
    let mut moof_body = mp4_box(*b"mfhd", &mfhd_body);
    moof_body.extend_from_slice(&mp4_box(*b"traf", &traf_body));
    let mut out = mp4_box(*b"moof", &moof_body);
    out.extend_from_slice(&mdat_box(8));
    out
}

fn trun_box(kind: TrunKind) -> Vec<u8> {
    let mut body = Vec::new();
    match kind {
        TrunKind::SampleDurations(durations) => {
            put_u32(&mut body, 0x0000_0101);
            put_u32(
                &mut body,
                u32::try_from(durations.len()).expect("fixture count fits u32"),
            );
            put_u32(&mut body, 0);
            for duration in durations {
                put_u32(&mut body, duration);
            }
        }
        TrunKind::CountOnly(count) => {
            put_u32(&mut body, 0x0000_0001);
            put_u32(&mut body, count);
            put_u32(&mut body, 0);
        }
    }
    mp4_box(*b"trun", &body)
}

fn moof_box(
    sequence: u32,
    tfdt_version: u8,
    tfdt_value: u64,
    tfhd_default: Option<u32>,
    trun: TrunKind,
) -> Vec<u8> {
    let mut mfhd_body = Vec::new();
    put_u32(&mut mfhd_body, 0);
    put_u32(&mut mfhd_body, sequence);
    let mut traf_body = tfhd_box(tfhd_default);
    traf_body.extend_from_slice(&tfdt_box(tfdt_version, tfdt_value));
    traf_body.extend_from_slice(&trun_box(trun));
    let mut moof_body = mp4_box(*b"mfhd", &mfhd_body);
    moof_body.extend_from_slice(&mp4_box(*b"traf", &traf_body));
    mp4_box(*b"moof", &moof_body)
}

fn mdat_box(payload_len: usize) -> Vec<u8> {
    let mut body = Vec::new();
    put_u32(
        &mut body,
        u32::try_from(payload_len).expect("fixture payload fits u32"),
    );
    body.extend_from_slice(&vec![0x5Au8; payload_len]);
    mp4_box(*b"mdat", &body)
}

fn fragment(durations: &[u32], tfdt_value: u64) -> Vec<u8> {
    let moof = moof_box(
        1,
        1,
        tfdt_value,
        None,
        TrunKind::SampleDurations(durations.to_vec()),
    );
    let mut out = moof;
    out.extend_from_slice(&mdat_box(durations.len() * 8 + 4));
    out
}

fn stored_piece(init: &[u8], fragments: &[&[u8]]) -> Vec<u8> {
    let mut out = init.to_vec();
    for fragment in fragments {
        out.extend_from_slice(fragment);
    }
    out
}

/// Byte index where one fixture `tfdt` value field sits inside its fragment bytes.
fn tfdt_field_of(fragment: &[u8]) -> usize {
    let marker = fragment
        .windows(4)
        .position(|window| window == *b"tfdt")
        .expect("fixture fragment carries a tfdt");
    marker + 8
}

fn one_fragment_piece(durations: &[u32]) -> Vec<u8> {
    let init = init_bytes(&default_init_spec());
    stored_piece(&init, &[&fragment(durations, 0)])
}

fn fingerprint_of(spec: &InitSpec) -> InitFingerprint {
    fingerprint(&init_bytes(spec)).expect("fixture init is readable")
}

fn fixture_parts(durations: &[u32]) -> corvette_api_server::rebase::PieceParts {
    split_piece(&one_fragment_piece(durations)).expect("fixture piece splits")
}

#[test]
fn split_reports_init_len_fragment_ranges_and_trun_durations() {
    let init = init_bytes(&default_init_spec());
    let first = fragment(&[40, 60], 0);
    let second = fragment(&[250], 100);
    let piece = stored_piece(&init, &[&first, &second]);
    let parts = split_piece(&piece).expect("fixture piece splits");

    assert_eq!(parts.init_len, init.len());
    assert_eq!(parts.fragments.len(), 2);
    assert_eq!(parts.fragments[0].start, init.len());
    assert_eq!(parts.fragments[0].end, init.len() + first.len());
    assert_eq!(parts.fragments[1].start, init.len() + first.len());
    assert_eq!(parts.fragments[1].end, piece.len());
    assert_eq!(parts.fragments[0].duration_ticks, 100);
    assert_eq!(parts.fragments[1].duration_ticks, 250);
    assert_eq!(parts.fragments[0].base_decode_ticks, 0);
    assert_eq!(parts.fragments[1].base_decode_ticks, 100);
}

#[test]
fn split_uses_the_tfhd_default_duration_when_the_trun_carries_none() {
    let init = init_bytes(&default_init_spec());
    let mut frag = moof_box(1, 1, 0, Some(33), TrunKind::CountOnly(2));
    frag.extend_from_slice(&mdat_box(16));
    let piece = stored_piece(&init, &[&frag]);
    let parts = split_piece(&piece).expect("default-duration piece splits");
    assert_eq!(parts.fragments[0].duration_ticks, 66);
}

#[test]
fn split_refuses_a_trun_without_durations_or_default() {
    let init = init_bytes(&default_init_spec());
    let mut frag = moof_box(1, 1, 0, None, TrunKind::CountOnly(2));
    frag.extend_from_slice(&mdat_box(16));
    assert!(matches!(
        split_piece(&stored_piece(&init, &[&frag])),
        Err(RebaseError::Malformed)
    ));
}

#[test]
fn split_reads_tfhd_default_duration_past_sample_description_index() {
    let mut fields = Vec::new();
    put_u64(&mut fields, 0);
    put_u32(&mut fields, 0x1111_1111);
    put_u32(&mut fields, 0x2222_2222);
    let frag = fragment_with_tfhd(
        &tfhd_with_raw_flags(0x0002_000B, &fields),
        TrunKind::CountOnly(2),
    );
    let parts = split_piece(&stored_piece(&init_bytes(&default_init_spec()), &[&frag]))
        .expect("indexed-default piece splits");
    assert_eq!(parts.fragments[0].duration_ticks, 2 * 0x2222_2222u64);
}

#[test]
fn split_reads_default_duration_with_each_tfhd_field_alone_and_together() {
    let duration: u32 = 0x2222_2222;
    let mut alone = Vec::new();
    put_u32(&mut alone, duration);
    let mut with_offset = Vec::new();
    put_u64(&mut with_offset, 0);
    put_u32(&mut with_offset, duration);
    let mut with_index = Vec::new();
    put_u32(&mut with_index, 0x1111_1111);
    put_u32(&mut with_index, duration);
    let mut with_size = Vec::new();
    put_u32(&mut with_size, duration);
    put_u32(&mut with_size, 0x3333_3333);
    let mut with_flags = Vec::new();
    put_u32(&mut with_flags, duration);
    put_u32(&mut with_flags, 0x0200_0000);
    let mut together = Vec::new();
    put_u64(&mut together, 0);
    put_u32(&mut together, 0x1111_1111);
    put_u32(&mut together, duration);
    put_u32(&mut together, 0x3333_3333);
    put_u32(&mut together, 0x0200_0000);
    let cases: [(u32, &[u8]); 6] = [
        (0x0000_0008, &alone),
        (0x0000_0009, &with_offset),
        (0x0000_000A, &with_index),
        (0x0000_0018, &with_size),
        (0x0000_0028, &with_flags),
        (0x0000_003B, &together),
    ];
    let init = init_bytes(&default_init_spec());
    for (flags, fields) in cases {
        let frag = fragment_with_tfhd(&tfhd_with_raw_flags(flags, fields), TrunKind::CountOnly(2));
        let parts = split_piece(&stored_piece(&init, &[&frag]))
            .unwrap_or_else(|error| panic!("flags {flags:#x}: {error}"));
        assert_eq!(
            parts.fragments[0].duration_ticks,
            u64::from(duration) * 2,
            "flags {flags:#x}"
        );
    }
}

#[test]
fn split_refuses_tfhd_flag_bits_outside_the_handled_set() {
    let init = init_bytes(&default_init_spec());
    for flags in [0x0002_0040u32, 0x0002_0080, 0x0004_0000, 0x0100_0000] {
        let frag = fragment_with_tfhd(
            &tfhd_with_raw_flags(flags, b""),
            TrunKind::SampleDurations(vec![10]),
        );
        assert!(
            matches!(
                split_piece(&stored_piece(&init, &[&frag])),
                Err(RebaseError::Malformed)
            ),
            "flags {flags:#x} must be refused"
        );
    }
    let short = tfhd_with_raw_flags(0x0002_0008, b"abc");
    let frag = fragment_with_tfhd(&short, TrunKind::CountOnly(1));
    assert!(matches!(
        split_piece(&stored_piece(&init, &[&frag])),
        Err(RebaseError::Malformed)
    ));
}

#[test]
fn split_refuses_a_trailing_box_after_the_fragment_pairs() {
    let init = init_bytes(&default_init_spec());
    let frag = fragment(&[10], 0);
    let mut with_free = stored_piece(&init, &[&frag]);
    with_free.extend_from_slice(&mp4_box(*b"free", b"pad"));
    assert!(matches!(
        split_piece(&with_free),
        Err(RebaseError::Malformed)
    ));
    let mut with_stray_mdat = stored_piece(&init, &[&frag]);
    with_stray_mdat.extend_from_slice(&mdat_box(8));
    assert!(matches!(
        split_piece(&with_stray_mdat),
        Err(RebaseError::Malformed)
    ));
}

#[test]
fn truncated_piece_at_every_cut_is_never_ok_with_wrong_ranges() {
    let init = init_bytes(&default_init_spec());
    let first = fragment(&[40, 60], 0);
    let second = fragment(&[250], 100);
    let piece = stored_piece(&init, &[&first, &second]);
    let one_fragment_end = init.len() + first.len();
    for cut in 0..piece.len() {
        let result = split_piece(&piece[..cut]);
        if cut == one_fragment_end {
            let parts = result.unwrap_or_else(|error| panic!("cut {cut} ends a fragment: {error}"));
            assert_eq!(parts.init_len, init.len());
            assert_eq!(parts.fragments.len(), 1);
            assert_eq!(
                (parts.fragments[0].start, parts.fragments[0].end),
                (init.len(), one_fragment_end)
            );
        } else {
            assert!(
                matches!(result, Err(RebaseError::Truncated | RebaseError::Malformed)),
                "cut {cut} returned {result:?}"
            );
        }
    }
}

#[test]
fn split_accepts_a_largesize_box_and_a_to_end_of_file_final_box() {
    let init = init_bytes(&default_init_spec());
    let moof = moof_box(1, 1, 0, None, TrunKind::SampleDurations(vec![10]));

    let mut large_piece = stored_piece(&init, &[]);
    large_piece.extend_from_slice(&moof);
    large_piece.extend_from_slice(&largesize_box(*b"mdat", &[7u8; 32]));
    let parts = split_piece(&large_piece).expect("largesize piece splits");
    assert_eq!(parts.fragments.len(), 1);
    assert_eq!(parts.fragments[0].end, large_piece.len());
    assert_eq!(parts.fragments[0].duration_ticks, 10);

    let mut final_piece = stored_piece(&init, &[]);
    final_piece.extend_from_slice(&moof);
    final_piece.extend_from_slice(&to_end_of_file_box(*b"mdat", &[9u8; 24]));
    let parts = split_piece(&final_piece).expect("to-end piece splits");
    assert_eq!(parts.fragments[0].end, final_piece.len());
}

#[test]
fn fingerprint_is_equal_for_inits_differing_only_in_creation_time() {
    let mut changed = default_init_spec();
    changed.creation_time = 1_733_000_000;
    assert_eq!(
        fingerprint_of(&default_init_spec()),
        fingerprint_of(&changed)
    );

    let mut extended = default_init_spec();
    extended.mdhd_version = 1;
    assert_eq!(
        fingerprint_of(&default_init_spec()),
        fingerprint_of(&extended),
        "the mdhd version widens the time fields, not the parameters"
    );
}

#[test]
fn fingerprint_differs_for_each_stream_parameter_change() {
    let baseline = fingerprint_of(&default_init_spec());

    let mut wider = default_init_spec();
    wider.width = 1280;
    assert_ne!(baseline, fingerprint_of(&wider), "width change");

    let mut taller = default_init_spec();
    taller.height = 720;
    assert_ne!(baseline, fingerprint_of(&taller), "height change");

    let mut reconfigured = default_init_spec();
    reconfigured.config_body = vec![0x01, 0x42, 0xC0, 0x20, 0xE1, 0xFF];
    assert_ne!(baseline, fingerprint_of(&reconfigured), "avcC change");

    let mut retimed = default_init_spec();
    retimed.timescale = 45_000;
    assert_ne!(baseline, fingerprint_of(&retimed), "timescale change");

    let mut recoded = default_init_spec();
    recoded.fourcc = *b"hvc1";
    recoded.config_kind = *b"hvcC";
    assert_ne!(
        baseline,
        fingerprint_of(&recoded),
        "sample-entry fourcc change"
    );
}

#[test]
fn fingerprint_refuses_an_init_the_walk_cannot_read() {
    assert!(matches!(fingerprint(&[]), Err(RebaseError::Malformed)));
    assert!(matches!(
        fingerprint(b"not an mp4 at all"),
        Err(RebaseError::Truncated | RebaseError::Malformed)
    ));
    let mut empty_moov = ftyp_box();
    empty_moov.extend_from_slice(&mp4_box(*b"moov", b""));
    assert!(matches!(
        fingerprint(&empty_moov),
        Err(RebaseError::Malformed)
    ));
}

#[test]
fn fingerprint_refuses_stsd_that_declares_other_than_one_entry() {
    let spec = default_init_spec();
    let mut entry_body = sample_entry_body(spec.width, spec.height);
    entry_body.extend_from_slice(&mp4_box(spec.config_kind, &spec.config_body));
    let entry = mp4_box(spec.fourcc, &entry_body);
    for count in [0u32, 2] {
        let mut body = Vec::new();
        put_u32(&mut body, 0);
        put_u32(&mut body, count);
        body.extend_from_slice(&entry);
        if count >= 2 {
            body.extend_from_slice(&entry);
        }
        let init = init_bytes_with_stsd(&spec, &body);
        assert!(
            matches!(fingerprint(&init), Err(RebaseError::Malformed)),
            "entry_count {count} must be refused"
        );
    }
}

#[test]
fn rebase_adds_offset_to_tfdt_and_changes_no_other_byte() {
    let frag = fragment(&[40, 60], 7);
    let rebased = rebase_fragment(&frag, 1_000).expect("rebase succeeds");
    assert_eq!(rebased.len(), frag.len());
    let field = tfdt_field_of(&frag);
    let changed: Vec<usize> = (0..frag.len())
        .filter(|i| rebased[*i] != frag[*i])
        .collect();
    assert!(!changed.is_empty(), "the rewrite must change something");
    assert!(
        changed.iter().all(|i| (field..field + 8).contains(i)),
        "only tfdt-field bytes may differ, got {changed:?}"
    );
    assert_eq!(&rebased[field..field + 8], &(7u64 + 1_000).to_be_bytes());
}

#[test]
fn rebase_with_zero_offset_returns_identical_bytes() {
    let frag = fragment(&[40, 60], 123_456_789);
    assert_eq!(rebase_fragment(&frag, 0).expect("identity rebase"), frag);
}

#[test]
fn rebase_overflow_is_refused() {
    let frag = fragment(&[10], u64::MAX - 5);
    assert!(matches!(
        rebase_fragment(&frag, 10),
        Err(RebaseError::Overflow)
    ));
    let exact = rebase_fragment(&frag, 5).expect("exact maximum is reachable");
    let field = tfdt_field_of(&frag);
    assert_eq!(&exact[field..field + 8], &u64::MAX.to_be_bytes());
}

#[test]
fn rebase_refuses_a_version_zero_tfdt_rather_than_widening_it() {
    // Every stored `tfdt` the bridge writes is version 1 (`fmp4.rs` `tfdt` writer),
    // so a version-0 field is foreign data, not a narrower encoding to widen.
    let mut frag = moof_box(1, 0, 7, None, TrunKind::SampleDurations(vec![10]));
    frag.extend_from_slice(&mdat_box(16));
    assert!(matches!(
        rebase_fragment(&frag, 1),
        Err(RebaseError::Malformed)
    ));
    let piece = stored_piece(&init_bytes(&default_init_spec()), &[&frag]);
    assert!(matches!(split_piece(&piece), Err(RebaseError::Malformed)));
}

#[test]
fn rebase_refuses_bytes_that_are_not_a_moof_mdat_pair() {
    assert!(matches!(
        rebase_fragment(b"", 0),
        Err(RebaseError::Truncated | RebaseError::Malformed)
    ));
    let only_moof = moof_box(1, 1, 0, None, TrunKind::SampleDurations(vec![10]));
    assert!(matches!(
        rebase_fragment(&only_moof, 0),
        Err(RebaseError::Malformed)
    ));
    let mut mdat_first = mdat_box(8);
    mdat_first.extend_from_slice(&only_moof);
    assert!(matches!(
        rebase_fragment(&mdat_first, 0),
        Err(RebaseError::Malformed)
    ));
}

#[test]
fn timeline_sums_durations_into_cumulative_offsets_in_one_run() {
    let fp = fingerprint_of(&default_init_spec());
    let mut timeline = Timeline::new();
    for durations in [vec![100u32], vec![200], vec![300]] {
        timeline
            .push(&fixture_parts(&durations), &fp, false)
            .expect("push");
    }
    let pieces = timeline.pieces();
    assert_eq!(pieces.len(), 3);
    assert_eq!(
        pieces.iter().map(|p| p.offset_ticks).collect::<Vec<_>>(),
        vec![0, 100, 300]
    );
    assert!(pieces.iter().all(|p| p.run_index == 0));
    assert!(!pieces.iter().any(|p| p.has_discontinuity_before));
    assert_eq!(timeline.run_first_pieces(), vec![0]);
}

#[test]
fn timeline_starts_a_new_run_with_a_discontinuity_at_a_parameter_change() {
    let fp_a = fingerprint_of(&default_init_spec());
    let mut wider = default_init_spec();
    wider.width = 1280;
    let fp_b = fingerprint_of(&wider);
    let piece_a = fixture_parts(&[100]);
    let piece_b = fixture_parts(&[200]);

    let mut timeline = Timeline::new();
    timeline.push(&piece_a, &fp_a, false).expect("push");
    timeline.push(&piece_b, &fp_a, false).expect("push");
    timeline.push(&piece_b, &fp_b, false).expect("push");
    timeline.push(&piece_a, &fp_a, false).expect("push");

    let pieces = timeline.pieces();
    assert_eq!(
        pieces.iter().map(|p| p.run_index).collect::<Vec<_>>(),
        vec![0, 0, 1, 2]
    );
    assert_eq!(
        pieces
            .iter()
            .map(|p| p.has_discontinuity_before)
            .collect::<Vec<_>>(),
        vec![false, false, true, true]
    );
    assert_eq!(
        pieces.iter().map(|p| p.offset_ticks).collect::<Vec<_>>(),
        vec![0, 100, 0, 0]
    );
    assert_eq!(timeline.run_first_pieces(), vec![0, 2, 3]);
}

#[test]
fn timeline_gap_restarts_the_offset_within_the_same_run() {
    let fp = fingerprint_of(&default_init_spec());
    let piece_100 = fixture_parts(&[100]);
    let piece_200 = fixture_parts(&[200]);

    let mut timeline = Timeline::new();
    timeline.push(&piece_100, &fp, false).expect("first push");
    timeline.push(&piece_200, &fp, true).expect("gap push");
    timeline.push(&piece_200, &fp, false).expect("third push");

    let pieces = timeline.pieces();
    assert_eq!(
        pieces
            .iter()
            .map(|p| p.has_discontinuity_before)
            .collect::<Vec<_>>(),
        vec![false, true, false]
    );
    assert_eq!(
        pieces.iter().map(|p| p.offset_ticks).collect::<Vec<_>>(),
        vec![0, 0, 200]
    );
    assert!(pieces.iter().all(|p| p.run_index == 0));
    assert_eq!(timeline.run_first_pieces(), vec![0]);
}

#[test]
fn timeline_marks_no_discontinuity_before_the_first_piece() {
    let fp = fingerprint_of(&default_init_spec());
    let parts = fixture_parts(&[100]);
    let mut timeline = Timeline::new();
    timeline.push(&parts, &fp, true).expect("push");
    assert!(!timeline.pieces()[0].has_discontinuity_before);
    assert_eq!(timeline.pieces()[0].offset_ticks, 0);
}

#[test]
fn timeline_refuses_offsets_that_exceed_64_bits_and_keeps_the_plan() {
    let fp = fingerprint_of(&default_init_spec());
    let mut frag = moof_box(1, 1, 0, Some(u32::MAX), TrunKind::CountOnly(u32::MAX));
    frag.extend_from_slice(&mdat_box(4));
    let piece = split_piece(&stored_piece(&init_bytes(&default_init_spec()), &[&frag]))
        .expect("huge piece splits");

    let mut timeline = Timeline::new();
    timeline.push(&piece, &fp, false).expect("first push");
    assert!(matches!(
        timeline.push(&piece, &fp, false),
        Err(RebaseError::Overflow)
    ));
    assert_eq!(timeline.pieces().len(), 1);
    assert_eq!(timeline.run_first_pieces(), vec![0]);
}

#[test]
fn timeline_refuses_the_overflowing_push_at_the_sum_and_stays_usable() {
    let fp = fingerprint_of(&default_init_spec());
    let mut frag = moof_box(1, 1, 0, Some(u32::MAX), TrunKind::CountOnly(u32::MAX));
    frag.extend_from_slice(&mdat_box(4));
    let huge = split_piece(&stored_piece(&init_bytes(&default_init_spec()), &[&frag]))
        .expect("huge piece splits");
    let huge_duration = huge.fragments[0].duration_ticks;
    let small = fixture_parts(&[1]);

    let mut timeline = Timeline::new();
    timeline.push(&huge, &fp, false).expect("first push");
    assert!(matches!(
        timeline.push(&huge, &fp, false),
        Err(RebaseError::Overflow)
    ));
    assert_eq!(timeline.pieces().len(), 1);
    timeline
        .push(&small, &fp, false)
        .expect("valid push after a refusal");
    assert_eq!(timeline.pieces()[1].offset_ticks, huge_duration);
    assert!(!timeline.pieces()[1].has_discontinuity_before);
}

#[test]
fn split_round_trips_a_rebased_two_fragment_piece() {
    let init = init_bytes(&default_init_spec());
    let first = fragment(&[10], 0);
    let second = fragment(&[20], 10);
    let piece = stored_piece(&init, &[&first, &second]);
    let before = split_piece(&piece).expect("original splits");

    let rebased_first = rebase_fragment(
        &piece[before.fragments[0].start..before.fragments[0].end],
        0,
    )
    .expect("rebase first");
    let rebased_second = rebase_fragment(
        &piece[before.fragments[1].start..before.fragments[1].end],
        5,
    )
    .expect("rebase second");
    let after = split_piece(&stored_piece(&init, &[&rebased_first, &rebased_second]))
        .expect("rebased piece splits");

    assert_eq!(after.init_len, before.init_len);
    let before_ranges: Vec<(usize, usize)> =
        before.fragments.iter().map(|f| (f.start, f.end)).collect();
    let after_ranges: Vec<(usize, usize)> =
        after.fragments.iter().map(|f| (f.start, f.end)).collect();
    assert_eq!(after_ranges, before_ranges);
    let before_durations: Vec<u64> = before.fragments.iter().map(|f| f.duration_ticks).collect();
    let after_durations: Vec<u64> = after.fragments.iter().map(|f| f.duration_ticks).collect();
    assert_eq!(after_durations, before_durations);
    assert_eq!(after.fragments[0].base_decode_ticks, 0);
    assert_eq!(after.fragments[1].base_decode_ticks, 15);
}

/// Small deterministic generator so the fuzz-style case never needs a dependency.
struct Xorshift(u64);

impl Xorshift {
    const fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0.wrapping_shl(13);
        self.0 ^= self.0.wrapping_shr(7);
        self.0 ^= self.0.wrapping_shl(17);
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(bound).expect("bound fits u64"))
            .unwrap_or(0)
    }
}

#[test]
fn generated_byte_strings_never_panic_any_of_the_three_reads() {
    let init = init_bytes(&default_init_spec());
    let first = fragment(&[40, 60], 0);
    let second = fragment(&[250], 100);
    let piece = stored_piece(&init, &[&first, &second]);
    let mut rng = Xorshift(0x2545_F491_4F6C_DD1D);
    let mut split_ok_count = 0usize;

    for round in 0..2_500 {
        let mut bytes: Vec<u8> = Vec::new();
        match round % 4 {
            0 | 1 => {
                let cut = if round < 2 {
                    piece.len()
                } else {
                    rng.below(piece.len() + 1)
                };
                bytes.extend_from_slice(&piece[..cut]);
            }
            2 => {
                bytes.extend_from_slice(&init);
                bytes.extend_from_slice(&rng.next_u64().to_be_bytes());
            }
            _ => {
                let len = rng.below(96);
                for _ in 0..len / 8 {
                    bytes.extend_from_slice(&rng.next_u64().to_be_bytes());
                }
            }
        }
        if let Ok(parts) = split_piece(&bytes) {
            split_ok_count += 1;
            assert!(parts.init_len <= parts.fragments.first().map_or(bytes.len(), |f| f.start));
            for window in parts.fragments.windows(2) {
                assert!(window[0].end <= window[1].start);
            }
            for part in &parts.fragments {
                assert!(part.end <= bytes.len());
            }
        }
        let _ = fingerprint(&bytes);
        let _ = rebase_fragment(&bytes, rng.next_u64());
    }
    assert!(split_ok_count >= 2, "fuzz corpus reaches valid pieces");
}
