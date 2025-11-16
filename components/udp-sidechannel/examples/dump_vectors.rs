use kaspa_core::time::unix_now;
use kaspa_udp_sidechannel::fixtures::{self, build_delta_vector, build_snapshot_vector, delta_fields, snapshot_fields};

fn main() {
    let out_dir = std::env::args().nth(1).expect("usage: cargo run --example dump_vectors -- <out_dir>");
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let ts = unix_now();
    let keypair = fixtures::default_keypair();

    let snapshot_fields = snapshot_fields(42, ts, true);
    let snapshot = build_snapshot_vector(fixtures::DEFAULT_SNAPSHOT_SEQ, fixtures::DEFAULT_SOURCE_ID, &snapshot_fields, &keypair)
        .into_datagram(0x01);
    let delta_fields = delta_fields(43, ts);
    let delta = build_delta_vector(fixtures::DEFAULT_SNAPSHOT_SEQ + 1, fixtures::DEFAULT_SOURCE_ID, &delta_fields, &keypair)
        .into_datagram(0x01);

    std::fs::write(format!("{out_dir}/snapshot.bin"), &snapshot).expect("write snapshot");
    std::fs::write(format!("{out_dir}/delta.bin"), &delta).expect("write delta");

    let mut bad_magic = snapshot.clone();
    bad_magic[0] ^= 0xFF;
    std::fs::write(format!("{out_dir}/bad_magic.bin"), &bad_magic).expect("write bad magic");

    let mut truncated = delta.clone();
    truncated.truncate(20);
    std::fs::write(format!("{out_dir}/truncated.bin"), &truncated).expect("write truncated");

    println!("seed datagrams written to {out_dir}");
}
