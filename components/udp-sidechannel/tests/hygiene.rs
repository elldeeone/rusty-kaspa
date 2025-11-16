use std::path::PathBuf;

#[test]
fn info_logs_do_not_include_payloads() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let files = ["src/service.rs", "src/digest/manager.rs"];
    for relative in files {
        let path = base.join(relative);
        let contents = std::fs::read_to_string(&path).expect("source readable");
        for (idx, line) in contents.lines().enumerate() {
            if line.contains("info!(") && (line.contains("payload") || line.contains("signature")) {
                panic!("info log leaks raw payloads: {}:{}", path.display(), idx + 1);
            }
        }
    }
}

#[test]
fn metric_reasons_are_bounded() {
    use kaspa_udp_sidechannel::frame::DropReason;

    let expected = [
        "crc",
        "version",
        "network_mismatch",
        "payload_cap",
        "fragment_timeout",
        "fec_incomplete",
        "queue_full",
        "duplicate",
        "stale_seq",
        "rate_cap",
        "signature",
    ];
    let actual: Vec<_> = DropReason::ALL.iter().map(|reason| reason.as_str()).collect();
    assert_eq!(actual, expected, "drop reason label set changed unexpectedly");
}
