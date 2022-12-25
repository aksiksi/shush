// From https://github.com/alexcrichton/cc-rs/blob/fba7feded71ee4f63cfe885673ead6d7b4f2f454/src/lib.rs#L2462
fn get_cpp_link_stdlib(target: &str) -> Option<&'static str> {
    if target.contains("msvc") {
        None
    } else if target.contains("apple") {
        Some("c++")
    } else if target.contains("freebsd") {
        Some("c++")
    } else if target.contains("openbsd") {
        Some("c++")
    } else if target.contains("android") {
        Some("c++_shared")
    } else {
        Some("stdc++")
    }
}

fn main() {
    let target = std::env::var("TARGET").unwrap();

    // Link C++ standard library.
    if let Some(cpp_stdlib) = get_cpp_link_stdlib(&target) {
        println!("cargo:rustc-link-lib=dylib={}", cpp_stdlib);
    }

    // Link macOS Accelerate framework for matrix calculations.
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=framework=Accelerate");
    }

    // On Windows, link against system libraries used by FFmpeg.
    if target.contains("msvc") {
        let ffmpeg_libs = [
            "strmiids.lib",
            "mf.lib",
            "mfplat.lib",
            "mfplay.lib",
            "mfreadwrite.lib",
            "mfuuid.lib",
            "dxva2.lib",
            "evr.lib",
            "vfw32.lib",
            "shlwapi.lib",
            "oleaut32.lib",
        ];
        for lib in ffmpeg_libs {
            println!("cargo:rustc-link-lib={}", lib);
        }
    }
}
