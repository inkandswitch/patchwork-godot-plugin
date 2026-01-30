use crate::{project::branch_db::HistoryRef};
use godot::{classes::{RefCounted, Resource, ResourceLoader, resource_loader::ThreadLoadStatus}, global, obj::Base, prelude::GodotClass};
use godot::prelude::*;
use rand::rng;
use samod::DocumentId;

#[derive(GodotClass, Debug)]
#[class(base=RefCounted)]
pub struct LazyLoadToken {
	base: Base<RefCounted>,
	original_path: Option<String>,
	path: String,
    resource: Option<Gd<Resource>>,
    failed: bool,
}

#[godot_api]
impl IRefCounted for LazyLoadToken {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            path: String::new(),
            original_path: None,
            resource: None,
            failed: false,
        }
    }
}

impl LazyLoadToken {
    pub fn new(path: String, original_path: Option<String>) -> Gd<LazyLoadToken> {
        let mut tok = Self::new_gd();
        tok.bind_mut().set_paths(path, original_path);
        tok
    }
    fn set_paths(&mut self, path: String, original_path: Option<String>) {
        self.path = path;
        self.original_path = original_path;
    }
}

#[godot_api]
impl LazyLoadToken {
    #[func]
    fn is_started(&self) -> bool {
        if self.failed || self.resource.is_some() {
            return true;
        }
        let status = ResourceLoader::singleton().load_threaded_get_status(&self.path);
        if status != ThreadLoadStatus::INVALID_RESOURCE {
            return true;
        }
        false
    }

    #[func]
    fn is_load_finished(&self) -> bool {
        if self.failed || self.resource.is_some() {
            return true;
        }
        let status = ResourceLoader::singleton().load_threaded_get_status(&self.path);
        if status == ThreadLoadStatus::LOADED || status == ThreadLoadStatus::FAILED {
            return true;
        }
        false
    }

    #[func]
    pub fn start_load(&mut self){
        if self.is_started() {
            return;
        }
        if ResourceLoader::singleton().load_threaded_request(&self.path) != global::Error::OK {
            self.failed = true;
        }
    }

    #[func]
    pub fn get_resource(&mut self) -> Option<Gd<Resource>> {
        if self.resource.is_some() {
            return self.resource.clone();
        }
        if !self.is_started() {
            self.start_load();
        }
        if self.failed {
            return None;
        }

        let res = ResourceLoader::singleton().load_threaded_get(&self.path);
        if let Some(mut res) = res {
            if let Some(original_path) = self.original_path.as_ref() {
                if &res.get_path().to_string() != original_path {
                    res.set_path_cache(original_path);
                }
            }
            self.resource = Some(res);
        } else {
            self.failed = true;
        }
        return self.resource.clone();
    }

    #[func]
    pub fn did_fail(&self) -> bool {
        self.failed
    }

    #[func]
    pub fn get_path(&self) -> GString {
        if let Some(original_path) = self.original_path.as_ref() {
            return GString::from(original_path);
        }
        GString::from(&self.path)
    }
}
