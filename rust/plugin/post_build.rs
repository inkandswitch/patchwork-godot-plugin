use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

//.lastbuild file format:
// OUT_DIR=...
// PROFILE=...
// TARGET=...

fn after_build(){
    // source the .lastbuild file
    let cwd = env::var("CRATE_MANIFEST_DIR").unwrap();
    let cwd_path = Path::new(&cwd);
    let lastbuild = fs::read_to_string(cwd_path.join("target/.lastbuild")).unwrap();
    
    // Parse the file contents
    let out_dir = lastbuild.split("OUT_DIR=").nth(1).unwrap().split("\n").next().unwrap();
    let profile = lastbuild.split("PROFILE=").nth(1).unwrap().split("\n").next().unwrap();
    let target = lastbuild.split("TARGET=").nth(1).unwrap().split("\n").next().unwrap();
    // cargo_manifest_dir is the directory of the Cargo.toml file
    println!("cargo:warning=OUT_DIR={}", out_dir);
    println!("cargo:warning=PROFILE={}", profile);
    println!("cargo:warning=TARGET={}", target);
    println!("cargo:warning=CWD={}", cwd);

    // Determine the platform name from the target triple
    let platform_name = if target.contains("darwin") {
        "macos"
    } else if target.contains("windows") {
        "windows"
    } else if target.contains("linux") {
        "linux"
    } else {
        panic!("Unsupported target platform: {}", target);
    };
    let target_dir = Path::new(&out_dir).parent().unwrap().parent().unwrap().parent().unwrap();
    let mut target_dirs = vec![target_dir.to_path_buf()];
    let mut targets = vec![target];
    // Get the library name and extension based on platform
    let (lib_name, lib_ext) = if target.contains("windows") {
        ("patchwork_rust_core", "dll")
    } else if target.contains("darwin") {
        ("libpatchwork_rust_core", "dylib")
    } else {
        ("libpatchwork_rust_core", "so")
    };
    // if this platform_name is macos, and the target is aarch64-apple-darwin, write a file to the target directory called ".nextbuild"
    // check if this exists first
    // if platform_name == "macos" {
    //     let new_target = if target == "aarch64-apple-darwin" {
    //         "x86_64-apple-darwin"
    //     } else {
    //         "aarch64-apple-darwin"
    //     };
    //     targets.push(new_target);
    //     // run cargo post build with all the arguments from the .lastbuild file, including the profile, except change the target to x86_64-apple-darwin
    //     // set the output directory to the target directory + /target triplet
    //     // target_dir is set to something like "rust/plugin/target/debug"
    //     // the new_Dir is gonna be "rust/plugin/target/x86_64-apple-darwin/debug"
    //     // so we need to get a
    //     let new_dir = target_dir.parent().unwrap().join(new_target).join(profile);
    //     let mut args = vec!["build", "--lib", "--target", new_target];
    //     // check if it's a release build, and if so, add the --release flag
    //     if profile == "release" {
    //         args.push("--release");
    //     }
    //     // just run `which cargo` to get the location of the cargo executable (and strip any newlines)
    //     let which_cargo = Command::new("which").arg("cargo").output().unwrap();
    //     let stdout = which_cargo.stdout;
    //     // it's expecting an &[u8], but we have a `Vec<u8>`, so we need to convert it
    //     let cargo_location = String::from_utf8_lossy(&stdout);
    //     let cargo_location = cargo_location.trim();
    //     println!("cargo:warning=Cargo location: {:?}", &cargo_location);
    //     println!("building to {:?}", &new_dir);
    //     // then just run cargo with those args
    //     let output = Command::new(&cargo_location)
    //         .args(&args)
    //         .current_dir(cwd)
    //         .output()
    //         .unwrap();
    //     println!("cargo:warning=Ran cargo post build with args: {:?}", args);
    //     println!("cargo:warning=Output:", );
    //     print!("{}", String::from_utf8_lossy(&output.stdout));
    //     println!("cargo:warning=Error:  ");
    //     print!("{}", String::from_utf8_lossy(&output.stderr));
    //     // check the exit code
    //     if output.status.success() {
    //         println!("cargo:warning=cargo post build succeeded");
    //     } else {
    //         panic!("cargo post build failed");
    //     }
    //     // push it back to the target_dirs vector
    
    //     target_dirs.push(new_dir);
    // }

    // for all the target_dirs, copy the library to the platform-specific directory
    let size = target_dirs.len();
    for (i, target_dir) in target_dirs.iter().enumerate() {
        // Construct paths
        let lib_path = target_dir.join(format!("{}.{}", lib_name, lib_ext));

        let platform_dir = Path::new(&cwd_path).join(platform_name);

        // Create platform directory if it doesn't exist
        fs::create_dir_all(&platform_dir).unwrap();
        println!("cargo:warning=target_dir directory {:?}", target_dir);
        // Copy the library to the platform-specific directory
        println!("cargo:warning=Copying library from {:?} to {:?}", lib_path, platform_dir);

        

        let dest_path = if platform_name == "macos" {
            // it goes in the "macos/libpatchwork_rust_core.macos.framework" directory
            // platform_dir.join(format!("{}.{}.{}", lib_name, targets[i], lib_ext))
            platform_dir.join(format!("{}.macos.framework", &lib_name)).join(format!("{}.{}", lib_name, lib_ext))
        } else {
            platform_dir.join(format!("{}.{}.{}", lib_name, targets[i], lib_ext))
        };
        // check that libpath exists
        if !lib_path.exists() {
            panic!("Library file does not exist: {:?}", lib_path);
        }
        fs::copy(&lib_path, &dest_path).unwrap();

        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:warning=Copied library to {:?}", dest_path);
    }
}

fn main() {
    // ensure this runs AFTER the build
    // println!("cargo:warning=Running after_build");
    // run the after_build function
    after_build();
}
