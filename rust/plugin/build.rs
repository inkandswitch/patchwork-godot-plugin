use std::env;
use std::fs;
use std::io;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
mod post_build;


fn get_git_describe() -> String {
	// run git describe --tags --abbrev=6
	let output = Command::new("git").args(&["describe", "--tags", "--abbrev=6"]).output().unwrap();
	let git_describe = String::from_utf8(output.stdout).unwrap_or_default().trim().to_string();
	return git_describe;
}

fn get_plugin_cfg_path() -> PathBuf {
	let cwd_dir = env::current_dir().unwrap();
	let plugin_cfg_dir = cwd_dir.parent().unwrap().parent().unwrap();
	let plugin_cfg_path = plugin_cfg_dir.join("plugin.cfg");
	return plugin_cfg_path;
}

fn update_plugin_cfg(){
	let plugin_cfg_path = get_plugin_cfg_path();
	if !plugin_cfg_path.exists() {
		println!("cargo:warning=plugin.cfg does not exist in the current directory");
		return;
	}
	let plugin_cfg = fs::read_to_string(plugin_cfg_path.clone());
	if plugin_cfg.is_err() {
		println!("cargo:warning=Failed to read plugin.cfg: {}", plugin_cfg.err().unwrap());
		return;
	}
	let plugin_cfg = plugin_cfg.unwrap();
	let git_describe = get_git_describe();
	println!("cargo:warning=git_describe={}", git_describe);
	// if it has more than two `-` in the version, replace all the subsequent `-` with `+`
	let mut version = git_describe.to_string();
	let first_index = git_describe.find("-");
	if let Some(first_index) = first_index {
		if first_index > 0 {
			version = version[..first_index + 1].to_string() + &version[first_index + 1..].replace("-", "+");
		}
	}
	println!("cargo:warning=plugin_cfg=\n{}", plugin_cfg);
	let lines = plugin_cfg.lines();
	let mut new_lines = Vec::new();
	for line in lines {
		if line.contains("version=") {
			new_lines.push(format!("version=\"{}\"", version));
			println!("cargo:warning=new_line={}", new_lines.last().unwrap());
		} else {
			new_lines.push(line.to_string());
		}
	}
	let new_plugin_cfg = new_lines.join("\n") + "\n";
	println!("cargo:warning=new_plugin_cfg=\n{}", new_plugin_cfg);
	// write the new plugin.cfg file
	fs::write(plugin_cfg_path, new_plugin_cfg).unwrap();
}

fn write_build_info(){
	update_plugin_cfg();
    let out_dir: String = env::var("OUT_DIR").unwrap();
    let profile = env::var("PROFILE").unwrap();
    let target = env::var("TARGET").unwrap();
    // the actual args passed to cargo (it's not in an environment variable, but it's passed as an argument to the build script)
    let args = env::args().collect::<Vec<String>>();
    println!("cargo:warning=OUT_DIR={}", out_dir);
    println!("cargo:warning=PROFILE={}", profile);
    println!("cargo:warning=TARGET={}", target);
    // write all these variables to a temporary file in the target directory
    let target_dir = Path::new(&out_dir).parent().unwrap().parent().unwrap().parent().unwrap().parent().unwrap();
    let temp_file = target_dir.join(".lastbuild");
    fs::write(&temp_file, format!("OUT_DIR={}\nPROFILE={}\nTARGET={}\nARGS={:?}", out_dir, profile, target, args)).unwrap();
    println!("cargo:warning=Wrote to {:?}", &temp_file);
}



fn main() {
    println!("cargo:warning=Running write_build_info");
    write_build_info();
	static_vcruntime::metabuild();
}
