use corvette_detect::revision;

#[test]
fn empty_input_hashes_to_the_published_vector() {
    assert_eq!(
        revision(b""),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn abc_hashes_to_the_published_vector() {
    assert_eq!(
        revision(b"abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn two_block_message_hashes_to_the_published_vector() {
    assert_eq!(
        revision(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "sha256:248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn fifty_five_byte_run_hashes_to_the_system_digest() {
    assert_eq!(
        revision(&[b'a'; 55]),
        "sha256:9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
    );
}

#[test]
fn fifty_six_byte_run_hashes_to_the_system_digest() {
    assert_eq!(
        revision(&[b'a'; 56]),
        "sha256:b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
    );
}

#[test]
fn sixty_four_byte_run_hashes_to_the_system_digest() {
    assert_eq!(
        revision(&[b'a'; 64]),
        "sha256:ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
    );
}

#[test]
fn one_million_bytes_hash_to_the_published_vector() {
    assert_eq!(
        revision(&vec![b'a'; 1_000_000]),
        "sha256:cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn digest_is_repeatable_and_changes_on_one_byte() {
    assert_eq!(revision(b"config a"), revision(b"config a"));
    assert_ne!(revision(b"config a"), revision(b"config b"));
}
