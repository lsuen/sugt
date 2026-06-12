fn main() {
    println!("cargo:rerun-if-env-changed=SUGT_PRODUCT");
    println!("cargo:rerun-if-env-changed=SUGT_BUILD_ID");
    println!("cargo:rerun-if-env-changed=SUGT_EDITION");
    println!("cargo:rerun-if-env-changed=SUGT_TRIAL_ENABLED");
    println!("cargo:rerun-if-env-changed=SUGT_TRIAL_EXPIRES_AT");
    tauri_build::build();
}
