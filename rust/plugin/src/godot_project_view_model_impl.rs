use std::time::{SystemTime, UNIX_EPOCH};

use automerge::ChangeHash;
use automerge_repo::DocumentId;

use crate::godot_project_api::SyncStatus;
use crate::godot_project_driver::InputEvent;
use crate::utils::{exact_human_readable_timestamp, human_readable_timestamp};
use crate::{godot_accessors::PatchworkConfigAccessor, godot_project_api::GodotProjectViewModel, utils::summarize_changes};
use crate::godot_project_impl::GodotProjectImpl;

impl GodotProjectViewModel for GodotProjectImpl {
	fn clear_project(&mut self) {
        self.stop();
		PatchworkConfigAccessor::set_user_value("user_name", "");
        PatchworkConfigAccessor::set_project_value("project_doc_id", "");
        PatchworkConfigAccessor::set_project_value("checked_out_branch_doc_id", "");
	}

	fn get_user_name(&self) -> String {
		PatchworkConfigAccessor::get_user_value("user_name", "Anonymous")
	}

    fn set_user_name(&self, name: String) {
		PatchworkConfigAccessor::set_user_value("user_name", &name);
		self.driver_input_tx
			.unbounded_send(InputEvent::SetUserName { name })
			.unwrap();
    }

	fn can_create_merge_preview_branch(&self) -> bool {
		match self.get_checked_out_branch_state() {
			Some(branch_state) => {
				!branch_state.is_main
			},
			_ => false,
		}
	}

	fn create_merge_preview_branch(&mut self) {
		let Some(checked_out_branch) = self.get_checked_out_branch_state() else {
			return;
		};
		let Some(fork_info) = &checked_out_branch.fork_info  else {
			return;
		};
		self.create_merge_preview_branch_between(
			checked_out_branch.doc_handle.document_id(),
			fork_info.forked_from.clone());
	}

    fn can_create_revert_preview_branch(&self, head: ChangeHash) -> bool {
		if self.preview_branch_active() { return false; }
		self.get_checked_out_branch_state().is_some()
	}
    fn create_revert_preview_branch(&mut self, head: ChangeHash) {
		let Some(checked_out_branch) = self.get_checked_out_branch_state() else {
			return;
		};
		self.create_revert_preview_branch_for(
			checked_out_branch.doc_handle.document_id(),
			vec![head]);
	}
    fn preview_branch_active(&self) -> bool {
		let branch_state = self.get_checked_out_branch_state();
		match branch_state {
			Some(state) =>
				state.merge_info.is_some() || state.revert_info.is_some(),
			_ => false
		}
	}
    fn confirm_preview_branch(&mut self) {
		let Some(branch_state) = self.get_checked_out_branch_state().cloned() else {
			return;
		};
		let Some(fork_info) = &branch_state.fork_info else {
			return;
		};
		if let Some(revert_info) = &branch_state.revert_info {
			self.delete_branch(branch_state.doc_handle.document_id());
			self.checkout_branch(fork_info.forked_from.clone());
			self.revert_to_heads(revert_info.reverted_to.clone());
		}
		else if let Some(merge_info) = branch_state.merge_info {
			self.merge_branch(branch_state.doc_handle.document_id(), merge_info.merge_into)
		}
	}
    fn discard_preview_branch(&mut self) {
		let Some(branch_state) = self.get_checked_out_branch_state().cloned() else {
			return;
		};
		let Some(fork_info) = &branch_state.fork_info else {
			return;
		};
		self.delete_branch(branch_state.doc_handle.document_id());
		self.checkout_branch(fork_info.forked_from.clone());
	}

	fn get_branch_history(&self) -> Vec<ChangeHash> {
		let Some(branch_state) = self.get_checked_out_branch_state().cloned() else {
			return vec!();
		};
		self.history.iter()
			.filter(|item|
				self.changes.get(item).is_some_and(|i|
					i.metadata.as_ref().is_some_and(|m|
						m.branch_id.as_ref().is_some_and(|id|
							*id == branch_state.doc_handle.document_id()))))
			.map(|hash| hash.clone())
			.collect::<Vec<ChangeHash>>()
	}

	fn get_change_username(&self, hash: ChangeHash) -> String {
		let Some(change) = self.changes.get(&hash) else {
			return "<ERROR>".to_string();
		};
		if let Some(meta) = &change.metadata {
			if let Some(author) = &meta.username {
				return author.clone();
			}
		};
		"Anonymous".to_string()
	}

	fn is_change_synced(&self, hash: ChangeHash) -> bool {
		let Some(change) = self.changes.get(&hash) else {
			return false;
		};
        change.synced
	}

	fn get_change_summary(&self, hash: ChangeHash) -> String {
		(|| {
			let change = self.changes.get(&hash)?;
			let meta = change.metadata.as_ref();
			let author = self.get_change_username(hash);

			// merge commit
			if let Some(merge_info) = &meta?.merge_metadata {
				let merged_branch = &self.get_branch_by_id(&merge_info.merged_branch_id)?.name;
				return Some(format!("↪️ {author} merged {merged_branch} branch"));
			}

			// revert commit
			if let Some(revert_info) = &meta?.reverted_to {
				let heads = revert_info.iter()
					.map(|s| &s[..7])
					.collect::<Vec<&str>>().join(", ");
				return Some(format!("↩️ {author} reverted to {heads}"));
			}

			// initial commit
			if self.is_change_setup(hash) {
				return Some(format!("Initialized repository"));
			}

			return Some(summarize_changes(&author, meta?.changed_files.as_ref()?));
		})().unwrap_or("Invalid data".to_string())
	}

	fn is_change_merge(&self, hash: ChangeHash) -> bool {
		let Some(change) = self.changes.get(&hash) else {
			return false;
		};
        let Some(meta) = &change.metadata else {
            return false;
        };
        return meta.merge_metadata.is_some();
	}

	fn is_change_setup(&self, hash: ChangeHash) -> bool {
		// TODO (Lilith's PR, important): Mark initial commits as initial in metadata instead of just counting
		return false;
	}

	fn get_change_exact_timestamp(&self, hash: ChangeHash) -> String {
		let Some(change) = self.changes.get(&hash) else {
			return "--".to_string();
		};
        exact_human_readable_timestamp(change.timestamp)
	}

	fn get_change_human_timestamp(&self, hash: ChangeHash) -> String {
		let Some(change) = self.changes.get(&hash) else {
			return "--".to_string();
		};
        human_readable_timestamp(change.timestamp)
	}

	fn get_change_merge_id(&self, hash: ChangeHash) -> Option<DocumentId> {
		Some(self.changes.get(&hash)?.metadata.as_ref()?.merge_metadata.as_ref()?.merged_branch_id.clone())
	}

    fn get_sync_status(&self) -> SyncStatus {
        let Some(info) = &self.sync_server_connection_info else {
            return SyncStatus::Unknown;
        };
        let Some(branch) = self.get_checked_out_branch_state() else {
            return SyncStatus::Unknown;
        };
        let Some(status) = info.docs.get(&branch.doc_handle.document_id()) else {
            return SyncStatus::Unknown;
        };
        let is_connected = info.last_received.is_some();
        if status.last_acked_heads.as_ref().unwrap() == &branch.synced_heads {
            if is_connected {
                return SyncStatus::UpToDate;
            }
            return SyncStatus::Disconnected(0);
        }

        if is_connected {
            return SyncStatus::Syncing;
        }

        let unsynced_count = self.changes
            .iter()
            .filter(|(_hash, c)|!c.synced)
            .count();

        return SyncStatus::Disconnected(unsynced_count)
    }

    fn print_sync_debug(&self) {
        let Some(info) = &self.sync_server_connection_info else {
            tracing::debug!("Sync info UNAVAILABLE!!!");
            return;
        };
        let is_connected = info.last_received.is_some();

        fn time(t: Option<SystemTime>) -> String {
            let Some(t) = t else {
                return "-".to_string()
            };
            human_readable_timestamp(t
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                .try_into().unwrap())
        }

        tracing::debug!("Sync info ===========================");
        tracing::debug!("is connected: {is_connected}");
        tracing::debug!("last received: {}", time(info.last_received));
        tracing::debug!("last sent: {}", time(info.last_sent));

        if let Some(branch) = self.get_checked_out_branch_state() {
            if let Some(status) = info.docs.get(&branch.doc_handle.document_id()) {
                tracing::debug!("\t{}:", branch.name);
                tracing::debug!("\tacked heads: {:?}", status.last_acked_heads);
                tracing::debug!("\tsent heads: {:?}", status.last_sent_heads);
                tracing::debug!("\tlast sent: {}", time(status.last_sent));
                tracing::debug!("\tlast sent: {}", time(status.last_received));
            }
        }
        tracing::debug!("=====================================");
    }
}
