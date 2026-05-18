use codeless_runtime::template::JobTemplate;
fn main() {
    let path = std::env::args().nth(1).expect("path arg required");
    let yaml = std::fs::read_to_string(&path).unwrap();
    match serde_yaml::from_str::<JobTemplate>(&yaml) {
        Ok(t) => println!("OK name={} stages={}", t.name, t.stages.len()),
        Err(e) => { eprintln!("PARSE FAIL: {e}"); std::process::exit(1); }
    }
}
