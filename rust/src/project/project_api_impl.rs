use std::time::{SystemTime, UNIX_EPOCH};

use automerge::ChangeHash;
use samod::DocumentId;
use tracing::instrument;

use crate::{diff::differ::ProjectDiff, helpers::utils::{BranchWrapper, CommitInfo, DiffWrapper, exact_human_readable_timestamp, human_readable_timestamp}, interop::godot_accessors::PatchworkConfigAccessor, project::{project::{CheckedOutBranchState, Project}, project_api::{BranchViewModel, ChangeViewModel, DiffViewModel, ProjectViewModel, SyncStatus}, project_driver::InputEvent}};

impl ProjectViewModel for Project {
	fn has_project(&self) -> bool {
		self.is_started()
	}

	fn get_project_id(&self) -> Option<DocumentId> {
		self.get_project_doc_id()
	}

	fn new_project(&mut self) {
		if self.is_started() {
			return;
		}
		self.start();
	}

	fn load_project(&mut self, id: &DocumentId) {
		if self.is_started() {
			return;
		}
		PatchworkConfigAccessor::set_project_value("project_doc_id", id.to_string().as_str());
		self.start();
	}

	fn clear_project(&mut self) {
		if !self.is_started() {
			return;
		}
        self.stop();
		PatchworkConfigAccessor::set_user_value("user_name", "");
        PatchworkConfigAccessor::set_project_value("project_doc_id", "");
        PatchworkConfigAccessor::set_project_value("checked_out_branch_doc_id", "");
	}

	fn has_user_name(&self) -> bool {
		PatchworkConfigAccessor::get_user_value("user_name", "") != ""
	}

	fn get_user_name(&self) -> String {
		PatchworkConfigAccessor::get_user_value("user_name", "Anonymous")
	}

    fn set_user_name(&self, name: String) {
		PatchworkConfigAccessor::set_user_value("user_name", &name);
		if self.is_started() {
			self.driver_input_tx
				.unbounded_send(InputEvent::SetUserName { name })
				.unwrap();
		}
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
			checked_out_branch.doc_handle.document_id().clone(),
			fork_info.forked_from.clone());
	}

    fn can_create_revert_preview_branch(&self, head: ChangeHash) -> bool {
		if self.is_revert_preview_branch_active() || self.is_merge_preview_branch_active() { return false; }
		if self.get_change(head).is_some_and(|c| !c.is_setup()) {
			return self.get_checked_out_branch_state().is_some();
		}
		false
	}
    fn create_revert_preview_branch(&mut self, head: ChangeHash) {
		let Some(checked_out_branch) = self.get_checked_out_branch_state() else {
			return;
		};
		self.create_revert_preview_branch_for(
			checked_out_branch.doc_handle.document_id().clone(),
			vec![head]);
	}

	fn is_revert_preview_branch_active(&self) -> bool {
		let branch_state = self.get_checked_out_branch_state();
		match branch_state {
			Some(state) =>
				state.revert_info.is_some(),
			_ => false
		}
	}

	fn is_merge_preview_branch_active(&self) -> bool {
		let branch_state = self.get_checked_out_branch_state();
		match branch_state {
			Some(state) =>
				state.merge_info.is_some(),
			_ => false
		}
	}

	fn is_safe_to_merge(&self) -> bool {
		let Some(current_branch) = self.get_checked_out_branch_state() else {
			return false;
		};
		let Some(merge_info) = current_branch.merge_info.as_ref() else {
			return false;
		};
		let Some(fork_info) = current_branch.fork_info.as_ref() else {
			return false;
		};

		let source_branch = self.branch_states.get(&fork_info.forked_from);
		let dest_branch = self.branch_states.get(&merge_info.merge_into);

		let Some(dest_branch) = dest_branch else {
			return false;
		};

		source_branch.is_some_and(|s|
			s.fork_info.as_ref().is_some_and(|i|
				i.forked_at == dest_branch.synced_heads))
	}

    fn confirm_preview_branch(&mut self) {
		let Some(branch_state) = self.get_checked_out_branch_state().cloned() else {
			return;
		};
		let Some(fork_info) = &branch_state.fork_info else {
			return;
		};
		if let Some(revert_info) = &branch_state.revert_info {
			self.delete_branch(branch_state.doc_handle.document_id().clone());
			self.checkout_branch(fork_info.forked_from.clone());
			self.revert_to_heads(revert_info.reverted_to.clone());
		}
		else if let Some(merge_info) = branch_state.merge_info {
			self.merge_branch(branch_state.doc_handle.document_id().clone(), merge_info.merge_into)
		}
	}
    fn discard_preview_branch(&mut self) {
		let Some(branch_state) = self.get_checked_out_branch_state().cloned() else {
			return;
		};
		let Some(fork_info) = &branch_state.fork_info else {
			return;
		};
		self.delete_branch(branch_state.doc_handle.document_id().clone());
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
							id == branch_state.doc_handle.document_id()))))
			.map(|hash| hash.clone())
			.collect::<Vec<ChangeHash>>()
	}

    fn get_sync_status(&self) -> SyncStatus {
		if !self.is_started() {
			// We have no reason to be connected, therefore just mark it as OK.
			return SyncStatus::UpToDate;
		}
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
        if status.last_acked_heads.as_ref().is_some_and(|s| s == &branch.synced_heads) {
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
		if !self.is_started() {
			return;
		}
        let Some(info) = &self.sync_server_connection_info else {
            tracing::debug!("Sync info UNAVAILABLE!!!");
            return;
        };
        let is_connected = info.last_received.is_some();

        // fn time(t: Option<SystemTime>) -> String {
        //     let Some(t) = t else {
        //         return "-".to_string()
        //     };
        //     human_readable_timestamp(t
        //         .duration_since(UNIX_EPOCH)
        //         .unwrap()
        //         .as_millis()
        //         .try_into().unwrap())
        // }

        tracing::debug!("Sync info ===========================");
        tracing::debug!("is connected: {is_connected}");
        tracing::debug!("last received: {:?}", info.last_received);
        tracing::debug!("last sent: {:?}", info.last_sent);

        if let Some(branch) = self.get_checked_out_branch_state() {
            if let Some(status) = info.docs.get(&branch.doc_handle.document_id()) {
                tracing::debug!("\t{}:", branch.name);
                tracing::debug!("\tacked heads: {:?}", status.last_acked_heads);
                tracing::debug!("\tsent heads: {:?}", status.last_sent_heads);
                tracing::debug!("\tlast sent: {:?}", status.last_sent);
                tracing::debug!("\tlast sent: {:?}", status.last_received);
            }
        }
        tracing::debug!("=====================================");
    }

	fn get_branch(&self, id: &DocumentId) -> Option<impl BranchViewModel + use<>> {
        let state = self
			.branch_states
			.get(&id)?;

		let mut children = self.branch_states
			.values()
			.filter(|b|
				b.fork_info.as_ref().is_some_and(|i|
					i.forked_from == id.clone()))
			.map(|b| b.doc_handle.document_id().clone())
			.collect::<Vec<DocumentId>>();

		children.sort_by(|a, b| {
			let a_state = self.branch_states.get(&a);
			let b_state = self.branch_states.get(&b);
			let Some(a_state) = a_state else {
				return std::cmp::Ordering::Less;
			};
			let Some(b_state) = b_state else {
				return std::cmp::Ordering::Greater;
			};
            a_state.name.to_lowercase().cmp(&b_state.name.to_lowercase())
        });

		Some(BranchWrapper {
			state: state.clone(),
			children
		})
	}

	fn get_main_branch(&self) -> Option<impl BranchViewModel> {
		let state = self
            .branch_states
            .values()
            .find(|branch_state| branch_state.is_main);
		self.get_branch(&state?.doc_handle.document_id())
	}

	fn get_checked_out_branch(&self) -> Option<impl BranchViewModel> {
        let state = self.get_checked_out_branch_state();
		self.get_branch(&state?.doc_handle.document_id())
	}

	#[instrument(skip(self), fields(name = ?name), level = tracing::Level::INFO)]
	fn create_branch(&mut self, name: String) {
		println!("");
		tracing::info!("******** CREATE BRANCH");
		println!("");
        let source_branch_doc_id = match &self.get_checked_out_branch_state() {
            Some(branch_state) => branch_state.doc_handle.document_id(),
            None => {
                panic!("couldn't create branch, no checked out branch");
            }
        };

        self.driver_input_tx
            .unbounded_send(InputEvent::CreateBranch {
                name,
                source_branch_doc_id: source_branch_doc_id.clone(),
            })
            .unwrap();

		// TODO: do we want to set this? or let _process set it?
        self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut(Some(source_branch_doc_id.clone()));
		// self.checked_out_branch_state = CheckedOutBranchState::NothingCheckedOut(None);
    }

	fn checkout_branch(&mut self, branch_doc_id: DocumentId) {
		let current_branch = match &self.checked_out_branch_state {
			CheckedOutBranchState::CheckedOut(doc_id, _) => Some(doc_id.clone()),
			CheckedOutBranchState::CheckingOut(doc_id, _) => {
				tracing::error!("**@#%@#%!@#%#@!*** CHECKING OUT BRANCH WHILE STILL CHECKING OUT?!?!?! {:?}", doc_id);
				Some(doc_id.clone())
			},
			CheckedOutBranchState::NothingCheckedOut(current_branch_id) => {
				tracing::warn!("Checking out a branch while not checked out on any branch????");
				current_branch_id.clone()
			}
		};
        let target_branch_state = match self.branch_states.get(&branch_doc_id) {
            Some(branch_state) => branch_state,
            None => panic!("couldn't checkout branch, branch doc id not found")
        };
		println!("");
		tracing::debug!("******** CHECKOUT: {:?}\n", target_branch_state.name);
		println!("");

        if target_branch_state.synced_heads == target_branch_state.doc_handle.with_document(|d| d.get_heads()) {
            self.checked_out_branch_state =
                CheckedOutBranchState::CheckedOut(
					branch_doc_id.clone(),
					current_branch.clone());
			self.just_checked_out_new_branch = true;
        } else {
			tracing::debug!("checked out branch {:?} has unsynced heads", target_branch_state.name);
            self.checked_out_branch_state =
				CheckedOutBranchState::CheckingOut(
					branch_doc_id.clone(),
					current_branch.clone()
				);
        }
    }

	fn get_change(&self, hash: ChangeHash) -> Option<&impl ChangeViewModel> {
		self.changes.get(&hash)
	}

	fn get_default_diff(&self) -> Option<impl DiffViewModel> {
		let heads_before;
		let heads_after;
		let branch_state = self.get_checked_out_branch_state()?;

		// There is no default diff for the main branch!
		if branch_state.is_main {
			return None;
		}

		if self.is_merge_preview_branch_active() {
			heads_before = branch_state.merge_info.as_ref()?.merge_at.clone();
		}
		// revert preview and regular branch both use forked_at
		else {
			heads_before = branch_state.fork_info.as_ref()?.forked_at.clone();
		}

		heads_after = branch_state.synced_heads.clone();

		// generate the summary
		let title;
		if self.is_merge_preview_branch_active() {
			let source_name = self.get_branch(&branch_state.fork_info.as_ref()?.forked_from.clone())?.get_name();
			let target_name = self.get_branch(&branch_state.merge_info.as_ref()?.merge_into.clone())?.get_name();
			title = format!("Showing changes for {} -> {}", source_name, target_name);
		}
		else if self.is_revert_preview_branch_active() {
			let source_name = self.get_branch(&branch_state.fork_info.as_ref()?.forked_from.clone())?.get_name();
			// assume reverted_to is always just 1 hash
			let short_heads = &branch_state.revert_info.as_ref()?.reverted_to
				.first()?.to_string()[..7];
			title = format!("Showing changes for {} reverted to {}", source_name, short_heads);
		}
		else {
			let source_name = self.get_branch(&branch_state.fork_info.as_ref()?.forked_from.clone())?.get_name();
			title = format!("Showing changes from {} -> {}", source_name, branch_state.name);
		}

		Some(DiffWrapper {
			diff: self.get_cached_diff(heads_before, heads_after),
			title
		})
	}

	fn get_diff(&self, selected_hash: ChangeHash) -> Option<impl DiffViewModel> {
		let change = self.changes.get(&selected_hash)?;
		if change.is_setup() {
			return None;
		}
		let heads_before;
		let heads_after = vec!(change.hash);

		let history = self.get_branch_history();
		let mut prev_hash = None;
		for (i, el) in history.iter().enumerate() {
			if *el == selected_hash {
				prev_hash = history.get(i - 1).copied();
				break;
			}
		}

		if let Some(prev_hash) = prev_hash {
			heads_before = vec!(prev_hash);
		}
		else {
			heads_before = self.get_checked_out_branch_state()?.fork_info.as_ref()?.forked_at.clone();
		}

		Some(DiffWrapper {
			diff: self.get_cached_diff(heads_before, heads_after),
			title: format!("Showing changes from {} - {}",
				change.get_summary(), change.get_human_timestamp())
		})
	}
}

impl ChangeViewModel for CommitInfo {
	fn get_hash(&self) -> ChangeHash {
		self.hash
	}

	fn get_username(&self) -> String {
		if let Some(meta) = &self.metadata {
			if let Some(author) = &meta.username {
				return author.clone();
			}
		};
		"Anonymous".to_string()
	}

	fn is_synced(&self) -> bool {
		self.synced
	}

	fn get_summary(&self) -> String {
		self.summary.clone()
	}

	fn is_merge(&self) -> bool {
        let Some(meta) = &self.metadata else {
            return false;
        };
        return meta.merge_metadata.is_some();
	}

	fn is_setup(&self) -> bool {
        let Some(meta) = &self.metadata else {
            return false;
        };
        return meta.is_setup.unwrap_or(false);
	}

	fn get_exact_timestamp(&self) -> String {
        exact_human_readable_timestamp(self.timestamp)
	}

	fn get_human_timestamp(&self) -> String {
        human_readable_timestamp(self.timestamp)
	}

	fn get_merge_id(&self) -> Option<DocumentId> {
		Some(self.metadata.as_ref()?
			.merge_metadata.as_ref()?
			.merged_branch_id.clone())
	}
}

impl BranchViewModel for BranchWrapper {
	fn get_id(&self) -> DocumentId {
		self.state.doc_handle.document_id().clone()
	}

	fn get_name(&self) -> String {
		self.state.name.clone()
	}

	fn get_parent(&self) -> Option<DocumentId> {
		Some(self.state.fork_info.as_ref()?.forked_from.clone())
	}

	fn get_children(&self) -> Vec<DocumentId> {
		self.children.clone()
	}

	fn is_available(&self) -> bool {
		!self.get_merge_into().is_some() && !self.get_reverted_to().is_some()
	}

	fn is_loaded(&self) -> bool {
        // we shouldn't have branches that don't have any changes but sometimes
        // the branch docs are not synced correctly so this flag is used in the UI to
        // indicate that the branch is not loaded and prevent users from checking it out
		!self.state.doc_handle.with_document(|d| d.get_heads().len() == 0)
	}

	fn get_reverted_to(&self) -> Option<ChangeHash> {
		Some(self.state.revert_info.as_ref()?.reverted_to.first()?.clone())
	}

	fn get_merge_into(&self) -> Option<DocumentId> {
		Some(self.state.merge_info.as_ref()?.merge_into.clone())
	}
}

impl DiffViewModel for DiffWrapper {
	fn get_diff(&self) -> &ProjectDiff {
		&self.diff
	}

	fn get_title(&self) -> &String {
		&self.title
	}
}
