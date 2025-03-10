use std::env;
use std::fs;
use std::path::Path;


fn write_build_info(){
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
}