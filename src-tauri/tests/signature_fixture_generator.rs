//! FAST_LOCAL: the committed G2 signature fixtures are exactly what the TEST ONLY generator
//! produces. Regenerate with `SIGNATURE_FIXTURES_WRITE=1 cargo test --test signature_fixture_generator`.

mod signature_support;

use signature_support::{fixtures_dir, g2_fixtures};

#[test]
fn committed_signature_fixtures_match_the_deterministic_generator() {
    let fixtures = g2_fixtures();
    let write = std::env::var_os("SIGNATURE_FIXTURES_WRITE").is_some();
    for (name, bytes) in &fixtures {
        let path = fixtures_dir().join(name);
        if write {
            std::fs::write(&path, bytes).unwrap();
            continue;
        }
        let committed = std::fs::read(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(
            committed == *bytes,
            "{name} differs from the generator output"
        );
    }
}

#[test]
fn the_generator_is_deterministic() {
    let first = g2_fixtures();
    let second = g2_fixtures();
    assert_eq!(first.len(), second.len());
    for ((name, left), (_, right)) in first.iter().zip(&second) {
        assert!(left == right, "{name} is not reproducible");
    }
}
