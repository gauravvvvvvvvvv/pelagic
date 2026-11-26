//! Reproducible source/dependency fingerprint for scene and run provenance.
fn main() {
    let mut files = vec![];
    fn visit(path: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(path).expect("source directory") {
            let p = e.unwrap().path();
            if p.is_dir() {
                visit(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    visit(std::path::Path::new("src"), &mut files);
    files.extend(["Cargo.toml", "Cargo.lock"].map(std::path::PathBuf::from));
    files.sort();
    let mut hash = 0xcbf29ce484222325u64;
    let compiler = std::process::Command::new(std::env::var_os("RUSTC").expect("compiler path"))
        .arg("--version")
        .output()
        .expect("compiler version");
    for byte in compiler
        .stdout
        .iter()
        .copied()
        .chain(std::env::var("TARGET").unwrap_or_default().bytes())
        .chain(std::env::var("PROFILE").unwrap_or_default().bytes())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for file in files {
        for byte in std::fs::read(&file).expect("build provenance input") {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        println!("cargo:rerun-if-changed={}", file.display());
    }
    println!("cargo:rustc-env=PELAGIC_BUILD_ID={hash:016x}");
}
