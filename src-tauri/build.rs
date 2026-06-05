fn main() {
    prepare_pi_sidecar();
    tauri_build::build()
}

fn prepare_pi_sidecar() {
    println!("cargo:rerun-if-changed=../scripts/prepare-pi-sidecar.mjs");
    println!("cargo:rerun-if-changed=binaries/pi-darwin-amd64.tar.xz");
    println!("cargo:rerun-if-changed=binaries/pi-darwin-arm64.tar.xz");
    println!("cargo:rerun-if-changed=binaries/pi-linux-amd64.tar.xz");
    println!("cargo:rerun-if-changed=binaries/pi-linux-arm64.tar.xz");
    println!("cargo:rerun-if-changed=binaries/pi-windows-amd64.zip");

    let Ok(target) = std::env::var("TARGET") else {
        return;
    };
    let output_name = match target.as_str() {
        "aarch64-apple-darwin" => "astra-aarch64-apple-darwin",
        "x86_64-apple-darwin" => "astra-x86_64-apple-darwin",
        "universal-apple-darwin" => "astra-universal-apple-darwin",
        "x86_64-unknown-linux-gnu" => "astra-x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu" => "astra-aarch64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc" => "astra-x86_64-pc-windows-msvc.exe",
        _ => return,
    };
    if std::path::Path::new("binaries").join(output_name).exists() {
        return;
    }

    let status = std::process::Command::new("node")
        .arg("../scripts/prepare-pi-sidecar.mjs")
        .arg(&target)
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("prepare-pi-sidecar failed with status {status}"),
        Err(error) => panic!("prepare-pi-sidecar failed: {error}"),
    }
}
