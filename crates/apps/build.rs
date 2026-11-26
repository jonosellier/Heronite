use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Copy the blacklist JSON to the output directory
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest_dir.join("..").join("..").join("data").join("app_blacklist.json");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dst = out_dir.join("app-blacklist.json");
    
    if src.exists() {
        fs::copy(&src, &dst).expect("Failed to copy app_blacklist.json");
        println!("cargo:rerun-if-changed={}", src.display());
    } else {
        panic!("Blacklist source file not found at: {}", src.display());
    }
}