use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

//.lastbuild file format:
// OUT_DIR=...
// PROFILE=...
// TARGET=...

// [configuration]
// entry_symbol = "gdext_rust_init"
// compatibility_minimum = 4.3
// reloadable = true

// [libraries]
// linux.debug.x86_64 =    "rust/plugin/linux/libpatchwork_rust_core.linux.x86_64-unknown-linux-gnu.so"
// linux.release.x86_64 =  "rust/plugin/linux/libpatchwork_rust_core.linux.x86_64-unknown-linux-gnu.so"
// linux.debug.arm64 =      "rust/plugin/linux/libpatchwork_rust_core.linux.aarch64-unknown-linux-gnu.so"
// linux.release.arm64 =    "rust/plugin/linux/libpatchwork_rust_core.linux.aarch64-unknown-linux-gnu.so"
// linux.debug.arm32 =      "rust/plugin/linux/libpatchwork_rust_core.linux.armv7-unknown-linux-gnueabihf.so"
// linux.release.arm32 =    "rust/plugin/linux/libpatchwork_rust_core.linux.armv7-unknown-linux-gnueabihf.so"
// windows.debug.x86_64 =   "rust/plugin/windows/patchwork_rust_core.windows.x86_64-pc-windows-msvc.dll"
// windows.release.x86_64 = "rust/plugin/windows/patchwork_rust_core.windows.x86_64-pc-windows-msvc.dll"
// windows.debug.arm64 =   "rust/plugin/windows/patchwork_rust_core.windows.aarch64-pc-windows-msvc.dll"
// windows.release.arm64 = "rust/plugin/windows/patchwork_rust_core.windows.aarch64-pc-windows-msvc.dll"
// macos.debug =           "rust/plugin/macos/libpatchwork_rust_core.macos.framework/libpatchwork_rust_core.macos.dylib"
// macos.release =         "rust/plugin/macos/libpatchwork_rust_core.macos.framework/libpatchwork_rust_core.macos.dylib"

// parse the above file and get the 


fn parse_gdextension_file(cwd: String) -> HashMap<String, String> {
    let gdext_path = Path::new(&cwd).join("Patchwork.gdextension");
    let contents = fs::read_to_string(gdext_path).expect("Failed to read Patchwork.gdextension");
    let mut map = HashMap::new();
    let mut current_section = String::new();
    let mut hit_libraries = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if line == "[libraries]" {
                hit_libraries = true;
            } else {
                hit_libraries = false;
            }
            if !hit_libraries {
                continue; // skip sections other than [libraries]
            }
            current_section = line[1..line.len()-1].to_string();
            continue;
        }
        if !hit_libraries {
            continue; // skip sections other than [libraries]
        }

        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

fn after_build(){
    // source the .lastbuild file
    let target_dir = env::var("CRATE_TARGET_DIR").unwrap();
    let crate_dir = env::var("CRATE_MANIFEST_DIR").unwrap();
    println!("cargo:warning=TARGET_DIR={}", target_dir);
    println!("cargo:warning=CWD={}", crate_dir);
    let crate_dir_path = Path::new(&crate_dir);
    let lastbuild = fs::read_to_string(Path::new(&target_dir).join(".lastbuild")).unwrap();

    // is_ok and is 1
    let isCI = env::var("CI").is_ok() && env::var("CI").unwrap() == "1";
    println!("cargo:warning=CI={}", isCI);

    // Parse the file contents
    let out_dir = lastbuild.split("OUT_DIR=").nth(1).unwrap().split("\n").next().unwrap();
    let profile = lastbuild.split("PROFILE=").nth(1).unwrap().split("\n").next().unwrap();
    let target = lastbuild.split("TARGET=").nth(1).unwrap().split("\n").next().unwrap();
    // cargo_manifest_dir is the directory of the Cargo.toml file
    println!("cargo:warning=OUT_DIR={}", out_dir);
    println!("cargo:warning=PROFILE={}", profile);
    println!("cargo:warning=TARGET={}", target);
    println!("cargo:warning=CWD={}", crate_dir);

    // arch is x86_64 or arm64 depending on the target
    let arch = if target.contains("x86_64") {
        "x86_64"
    } else if target.contains("arm64") || target.contains("aarch64") {
        "arm64"
    } else {
        panic!("Unsupported target architecture: {}", target);
    };

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
    let profile_dir = Path::new(&out_dir).parent().unwrap().parent().unwrap().parent().unwrap();
    let mut target_dirs = vec![profile_dir.to_path_buf()];
    let mut targets = vec![target];
    // Get the library name and extension based on platform
    let (lib_name, lib_dll_ext, lib_a_ext) = if target.contains("windows") {
        ("patchwork_rust_core", "dll", "lib")
    } else if target.contains("darwin") {
        ("libpatchwork_rust_core", "dylib", "a")
    } else {
        ("libpatchwork_rust_core", "so", "a")
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
    //     // profile_dir is set to something like "target/debug"
    //     // the new_Dir is gonna be "target/x86_64-apple-darwin/debug"
    //     // so we need to get a
    //     let new_dir = profile_dir.parent().unwrap().join(new_target).join(profile);
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
    //         .current_dir(crate_dir)
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
    // let map = parse_gdextension_file(crate_dir.clone());
    // let files = map.iter().map(|(k, v)| {
    //     if k.starts_with(&format!("{}.{}", platform_name.clone(), profile.to_ascii_lowercase())) {
    //         // e.g. linux.debug.x86_64
    //         if (platform_name == "macos"){
    //             return Some(   v);
    //         } else if (k.ends_with(arch)) {
    //             return Some(v);
    //         }
    //         // let parts: Vec<&str> = k.split('.').collect();
    //         // if parts.len() >= 3 {
    //         //     let target_part = parts[2]; // e.g. x86_64
    //         //     targets.push(target_part);
    //         //     let new_dir = profile_dir.parent().unwrap().join(target_part).join(profile);
    //         //     target_dirs.push(new_dir);
    //         // }
    //     }
    //     return None;
    // });
    // for all the target_dirs, copy the library to the platform-specific directory
    let size = target_dirs.len();
    for (i, profile_dir) in target_dirs.iter().enumerate() {
        // Construct paths
        let dll_lib_path = profile_dir.join(format!("{}.{}", lib_name, lib_dll_ext));
        let a_lib_path = profile_dir.join(format!("{}.{}", lib_name, lib_a_ext));

        let platform_dir = Path::new(&crate_dir_path).join(platform_name);

        // Create platform directory if it doesn't exist
        fs::create_dir_all(&platform_dir).unwrap();
        println!("cargo:warning=profile_dir directory {:?}", profile_dir);
        // Copy the library to the platform-specific directory
        println!("cargo:warning=Copying library from {:?} to {:?}", dll_lib_path, platform_dir);
        println!("cargo:warning=Copying library from {:?} to {:?}", a_lib_path, platform_dir);


        let dll_dest_path = if platform_name == "macos" {
            // it goes in the "macos/libpatchwork_rust_core.macos.framework" directory
            // platform_dir.join(format!("{}.{}.{}", lib_name, targets[i], lib_dll_ext))
            // if isCI {
            //     platform_dir.join(format!("{}.macos.framework", &lib_name)).join(format!("{}.{}", lib_name, lib_dll_ext))
            // } else {
                platform_dir.join(format!("{}.macos.framework", &lib_name)).join(format!("{}.macos.{}", lib_name, lib_dll_ext))
            // }
        } else {
            platform_dir.join(format!("{}.{}.{}.{}", lib_name, platform_name, targets[i], lib_dll_ext))
        };
        let a_dest_path = if platform_name == "macos" {
            platform_dir.join(format!("{}.macos.framework", &lib_name)).join(format!("{}.macos.{}.{}", lib_name, arch, lib_a_ext))
        } else {
            platform_dir.join(format!("{}.{}.{}.{}", lib_name, platform_name, targets[i], lib_a_ext))
        };
        // check that libpath exists
        if !dll_lib_path.exists() {
            panic!("Library file does not exist: {:?}", dll_lib_path);
        }
        if !a_lib_path.exists() {
            panic!("Library file does not exist: {:?}", a_lib_path);
        }
        fs::copy(&dll_lib_path, &dll_dest_path).unwrap();
        fs::copy(&a_lib_path, &a_dest_path).unwrap();

        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:warning=Copied library to {:?}", dll_dest_path);
        println!("cargo:warning=Copied library to {:?}", a_dest_path);
		// if on macos, run rcodesign
		if platform_name == "macos" {
			// framework path
			//codesign --force --deep --verbose -s "Apple Development: Nikita Zatkovich (RFTZV7M2RV)" macos/libpatchwork_rust_core.macos.framework
			let framework_path = platform_dir.join(format!("{}.macos.framework", &lib_name));
			let cwd = crate_dir_path.to_str().unwrap();
			// check that ".cargo/.devidentity" exists
			let mut dev_identity_path = Path::new(&crate_dir_path).join(".cargo/.devidentity");
			if !dev_identity_path.exists() {
				// try ../../
				dev_identity_path = Path::new(&crate_dir_path).parent().unwrap().parent().unwrap().join(".cargo/.devidentity");
				if !dev_identity_path.exists() {
					println!("cargo:warning=Developer identity file does not exist: {:?}", dev_identity_path);
					println!("Put the identity (e.g. Apple Development: Nikita Zatkovich (RFTZV7M2RV)) in the .cargo/.devidentity file to enable codesigning");
					return;
				}
			}
			// load the identity from the file
			let identity = fs::read_to_string(dev_identity_path).unwrap().trim().to_string();
			// let mut args = vec!["sign", framework_path.to_str().unwrap(), "--exclude", "**/*.a"];
			// let output = Command::new("rcodesign").args(&args).current_dir(cwd).output().unwrap();
			let mut args = vec![ "--force", "--deep", "--verbose", "-s", &identity, framework_path.to_str().unwrap()];
			println!("cargo:warning=codesigning with identity: {}", &identity);
			let output = Command::new("codesign").args(&args).current_dir(cwd).output().unwrap();

			println!("cargo:warning=rcodesign output: {}", String::from_utf8_lossy(&output.stdout));
			println!("cargo:warning=rcodesign error: {}", String::from_utf8_lossy(&output.stderr));
		}
    }
}

fn main() {
    // ensure this runs AFTER the build
    // println!("cargo:warning=Running after_build");
    // run the after_build function
    after_build();
}
