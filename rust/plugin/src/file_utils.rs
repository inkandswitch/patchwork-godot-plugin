use std::fs::File;
use std::io;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::str;
use ya_md5::{Md5Hasher, Hash, Md5Error};

use crate::godot_parser::{GodotScene, recognize_scene, parse_scene};

#[derive(Debug, Clone, PartialEq)]
pub enum FileContent {
    String(String),
    Binary(Vec<u8>),
    Scene(GodotScene),
}

impl FileContent {

	// Write file content to disk
	pub fn write_file_content(path: &PathBuf, content: &FileContent) -> std::io::Result<String> {
		// Check if the file exists
		let mut _temp_text: Option<String> = None;
		// Write the content based on its type
		let buf: &[u8] = match content {
			FileContent::String(text) => {
				text.as_bytes()
			}
			FileContent::Binary(data) => {
				data
			}
			FileContent::Scene(scene) => {
				_temp_text = Some(scene.serialize());
				_temp_text.as_ref().unwrap().as_bytes()
			}
		};
		let hash = Md5Hasher::hash_slice(buf).to_string();
		// Open the file with the appropriate mode
		let mut file = if path.exists() {
			// If file exists, open it for writing (truncate)
			File::options().write(true).truncate(true).open(path)?
		} else {
			// If file doesn't exist, create it
			File::create(path)?
		};
		let result = file.write_all(buf);
		if result.is_err() {
			return Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to write file"));
		}
		Ok(hash)
	}

	// pub fn from_path(path: &PathBuf) -> Option<FileContent> {
	// 	let hash = calculate_file_hash(path);
	// 	if hash.is_none() {
	// 		return FileContent::Binary(std::fs::read(path).unwrap());
	// 	}
	// 	FileContent::String(hash.unwrap())
	// }

	pub fn from_string(string: String) -> FileContent {
		// check if the file is a scene or a tres
		if recognize_scene(&string) {
			let scene = parse_scene(&string);
			if scene.is_ok() {
				return FileContent::Scene(scene.unwrap());
			}
		}
		FileContent::String(string)
	}

	pub fn from_buf(buf: Vec<u8>) -> FileContent {
		// check the first 8000 bytes (or the entire file if it's less than 8000 bytes) for a null byte
		if is_buf_binary(&buf) {
			return FileContent::Binary(buf);
		}
		let str = str::from_utf8(&buf);
		if str.is_err() {
			return FileContent::Binary(buf);
		}
		let string = str.unwrap().to_string();
		FileContent::from_string(string)
	}

}

pub fn calculate_file_hash(path: &PathBuf) -> Option<String> {
	if !path.is_file() {
		return None;
	}

	let mut file = match File::open(path) {
		Ok(file) => file,
		Err(_) => return None,
	};

	match Md5Hasher::hash(&mut file) {
		Ok(hash) => Some(format!("{}", hash)),
		Err(_) => None,
	}
}

// get the buffer and hash of a file
pub fn get_buffer_and_hash(path: &PathBuf) -> Result<(Vec<u8>, String), io::Error> {
	if !path.is_file() {
		return Err(io::Error::new(io::ErrorKind::Other, "Not a file"));
	}
	let buf = std::fs::read(path);
	if buf.is_err() {
		return Err(io::Error::new(io::ErrorKind::Other, "Failed to read file"));
	}
	let buf = buf.unwrap();
	let hash = Md5Hasher::hash_slice(&buf);
	let hash_str = format!("{}", hash);
	Ok((buf, hash_str))
}

pub fn is_file_binary(path: &PathBuf) -> bool {
	if !path.is_file() {
		return false;
	}

	let mut file = match File::open(path) {
		Ok(file) => file,
		Err(_) => return false,
	};

	// check the first 8000 bytes for a null byte
	let mut buffer = [0; 8000];
	if file.read(&mut buffer).is_err() {
		return false;
	}
	return is_buf_binary(&buffer);
}

pub fn is_buf_binary(buf: &[u8]) -> bool {
	buf.iter().take(8000).filter(|&b| *b == 0).count() > 0
}
