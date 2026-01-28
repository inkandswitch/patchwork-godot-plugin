use std::time::{SystemTime, UNIX_EPOCH};

use automerge::ChangeHash;
use samod::DocumentId;
use tracing::instrument;

use crate::{
    diff::differ::ProjectDiff,
    helpers::utils::{
        BranchWrapper, CommitInfo, DiffWrapper, exact_human_readable_timestamp,
        human_readable_timestamp,
    },
    interop::godot_accessors::PatchworkConfigAccessor,
    project::{
        branch_db::HistoryRef,
        project::Project,
        project_api::{
            BranchViewModel, ChangeViewModel, DiffViewModel, ProjectViewModel, SyncStatus,
        },
    },
};

// TODO: Ideally this is actually a child of a new project submodule...
// that's so that it doesn't need pub(super) to acess private fields of
// itself.

// TODO (Lilith): Figure out if there's a reasonable way to reduce blocking in this file.
// In general I kind of hate this, but I guess a sync/async divide is never going to look pretty.
impl ProjectViewModel for Project {
    fn has_project(&self) -> bool {
        self.driver.blocking_lock().is_some()
    }

    fn get_project_id(&self) -> Option<DocumentId> {
        self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return None;
            }
            driver.as_ref().unwrap().get_metadata_doc().await
        })
    }

    fn new_project(&mut self) {
        if self.has_project() {
            return;
        }
        self.start();
    }

    fn load_project(&mut self, id: &DocumentId) {
        if self.has_project() {
            return;
        }
        PatchworkConfigAccessor::set_project_value("project_doc_id", id.to_string().as_str());
        self.start();
    }

    fn clear_project(&mut self) {
        if !self.has_project() {
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
        self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return;
            }
            driver.as_ref().unwrap().set_username(Some(name)).await
        });
    }

    fn can_create_merge_preview_branch(&self) -> bool {
        match self.get_checked_out_branch_state() {
            Some(branch_state) => !branch_state.is_main,
            _ => false,
        }
    }

    fn create_merge_preview_branch(&mut self) {
        let Some(checked_out_branch) = self.get_checked_out_branch_state() else {
            return;
        };
        let Some(fork_info) = &checked_out_branch.fork_info else {
            return;
        };

        let source = checked_out_branch.doc_handle.document_id().clone();
        let target = fork_info.forked_from.clone();
        self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return;
            }
            driver
                .as_ref()
                .unwrap()
                .create_merge_preview_branch(&source, &target)
                .await;
        });
    }

    fn can_create_revert_preview_branch(&self, head: ChangeHash) -> bool {
        // TODO (Lilith): implement
        return false;
        // if self.is_revert_preview_branch_active() || self.is_merge_preview_branch_active() {
        //     return false;
        // }
        // if self.get_change(head).is_some_and(|c|
        // 	// Allow reverts for only the second setup commit
        // 	!c.is_setup() || self.get_branch_history().iter().position(|&h| h == head) == Some(1))
        // {
        //     return self.get_checked_out_branch_state().is_some();
        // }
        // false
    }
    fn create_revert_preview_branch(&mut self, head: ChangeHash) {
        // TODO (Lilith): implement
        return;
        // let Some(checked_out_branch) = self.get_checked_out_branch_state() else {
        //     return;
        // };
        // self.create_revert_preview_branch_for(
        //     checked_out_branch.doc_handle.document_id().clone(),
        //     vec![head],
        // );
    }

    fn is_revert_preview_branch_active(&self) -> bool {
        // TODO (Lilith): implement
        return false;
        // let branch_state = self.get_checked_out_branch_state();
        // match branch_state {
        //     Some(state) => state.revert_info.is_some(),
        //     _ => false,
        // }
    }

    fn is_merge_preview_branch_active(&self) -> bool {
        let branch_state = self.get_checked_out_branch_state();
        match branch_state {
            Some(state) => state.merge_info.is_some(),
            _ => false,
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

        let forked_from = fork_info.forked_from.clone();
        let merge_into = merge_info.merge_into.clone();
        let Some((source_branch, dest_branch)) = self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return None;
            }
            let source_branch = driver
                .as_ref()
                .unwrap()
                .get_branch_state(&forked_from)
                .await;
            let dest_branch = driver.as_ref().unwrap().get_branch_state(&merge_into).await;
            Some((source_branch, dest_branch))
        }) else {
            return false;
        };

        let Some(dest_branch) = dest_branch else {
            return false;
        };

        source_branch.is_some_and(|s| {
            s.fork_info
                .as_ref()
                .is_some_and(|i| i.forked_at == dest_branch.synced_heads)
        })
    }

    fn confirm_preview_branch(&mut self) {
        let Some(branch_state) = self.get_checked_out_branch_state() else {
            return;
        };
        let Some(fork_info) = &branch_state.fork_info else {
            return;
        };
        if let Some(revert_info) = &branch_state.revert_info {
            // TODO (Lilith): Implement
            // self.delete_branch(branch_state.doc_handle.document_id().clone());
            // self.checkout_branch(fork_info.forked_from.clone());
            // self.revert_to_heads(revert_info.reverted_to.clone());
        } else if let Some(merge_info) = branch_state.merge_info {
            let source = branch_state.doc_handle.document_id().clone();
            let target = merge_info.merge_into;
            self.with_driver_blocking(|driver| async move {
                if driver.is_none() {
                    return;
                }
                driver
                    .as_ref()
                    .unwrap()
                    .merge_branch(&source, &target)
                    .await;
            });
        }
    }
    fn discard_preview_branch(&mut self) {
        let Some(branch_state) = self.get_checked_out_branch_state() else {
            return;
        };
        let Some(fork_info) = &branch_state.fork_info else {
            return;
        };
        self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return;
            }
            driver.as_ref().unwrap().discard_current_branch().await;
        });
    }

    fn get_branch_history(&self) -> Vec<ChangeHash> {
        self.history.clone()
    }

    fn get_sync_status(&self) -> SyncStatus {
        // TODO (Lilith): implement
        return SyncStatus::Unknown;
        // if !self.has_project() {
        //     // We have no reason to be connected, therefore just mark it as OK.
        //     return SyncStatus::UpToDate;
        // }

        // let Some(info) = &self.sync_server_connection_info else {
        //     return SyncStatus::Unknown;
        // };
        // let Some(branch) = self.get_checked_out_branch_state() else {
        //     return SyncStatus::Unknown;
        // };
        // let Some(status) = info.docs.get(&branch.doc_handle.document_id()) else {
        //     return SyncStatus::Unknown;
        // };
        // let is_connected = info.last_received.is_some();
        // if status
        //     .last_acked_heads
        //     .as_ref()
        //     .is_some_and(|s| s == &branch.synced_heads)
        // {
        //     if is_connected {
        //         return SyncStatus::UpToDate;
        //     }
        //     return SyncStatus::Disconnected(0);
        // }

        // if is_connected {
        //     return SyncStatus::Syncing;
        // }

        // let unsynced_count = self.changes.iter().filter(|(_hash, c)| !c.synced).count();

        // return SyncStatus::Disconnected(unsynced_count);
    }

    fn print_sync_debug(&self) {
        // TODO (Lilith): implement
        return;
        // if !self.is_started() {
        //     return;
        // }
        // let Some(info) = &self.sync_server_connection_info else {
        //     tracing::debug!("Sync info UNAVAILABLE!!!");
        //     return;
        // };
        // let is_connected = info.last_received.is_some();

        // // fn time(t: Option<SystemTime>) -> String {
        // //     let Some(t) = t else {
        // //         return "-".to_string()
        // //     };
        // //     human_readable_timestamp(t
        // //         .duration_since(UNIX_EPOCH)
        // //         .unwrap()
        // //         .as_millis()
        // //         .try_into().unwrap())
        // // }

        // tracing::debug!("Sync info ===========================");
        // tracing::debug!("is connected: {is_connected}");
        // tracing::debug!("last received: {:?}", info.last_received);
        // tracing::debug!("last sent: {:?}", info.last_sent);

        // if let Some(branch) = self.get_checked_out_branch_state() {
        //     if let Some(status) = info.docs.get(&branch.doc_handle.document_id()) {
        //         tracing::debug!("\t{}:", branch.name);
        //         tracing::debug!("\tacked heads: {:?}", status.last_acked_heads);
        //         tracing::debug!("\tsent heads: {:?}", status.last_sent_heads);
        //         tracing::debug!("\tlast sent: {:?}", status.last_sent);
        //         tracing::debug!("\tlast sent: {:?}", status.last_received);
        //     }
        // }
        // tracing::debug!("=====================================");
    }

    fn get_branch(&self, id: &DocumentId) -> Option<impl BranchViewModel + use<>> {
        let id = id.clone();

        let Some((state, mut children)) = self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return None;
            }
            let Some(state) = driver.as_ref().unwrap().get_branch_state(&id).await else {
                return None;
            };
            let children = driver.as_ref().unwrap().get_branch_children(&id).await;
            Some((state, children))
        }) else {
            return None;
        };

        children.sort_by(|a, b| {
            let a_state = self.get_branch(a);
            let b_state = self.get_branch(b);
            let Some(a_state) = a_state else {
                return std::cmp::Ordering::Less;
            };
            let Some(b_state) = b_state else {
                return std::cmp::Ordering::Greater;
            };
            a_state
                .get_name()
                .to_lowercase()
                .cmp(&b_state.get_name().to_lowercase())
        });

        Some(BranchWrapper {
            state: state.clone(),
            // children,
            children: Vec::new(),
        })
    }

    fn get_main_branch(&self) -> Option<impl BranchViewModel> {
        let driver = self.driver.clone();
        let id = self
            .runtime
            .block_on(self.runtime.spawn(async move {
                let driver = driver.lock().await;
                if driver.is_none() {
                    return None;
                }
                driver.as_ref().unwrap().get_main_branch().await
            }))
            .unwrap()?;
        self.get_branch(&id)
    }

    fn get_checked_out_branch(&self) -> Option<impl BranchViewModel> {
        let driver = self.driver.clone();
        let id = self
            .runtime
            .block_on(self.runtime.spawn(async move {
                let driver = driver.lock().await;
                if driver.is_none() {
                    return None;
                }
                driver.as_ref().unwrap().get_checked_out_ref().await
            }))
            .unwrap()?;
        self.get_branch(&id.branch)
    }

    fn create_branch(&mut self, name: String) {
        let Some(branch_state) = self.get_checked_out_branch_state() else {
            return;
        };
        self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return;
            }
            driver
                .as_ref()
                .unwrap()
                .fork_branch(name, branch_state.doc_handle.document_id()).await;
        });
    }

    fn checkout_branch(&mut self, branch_doc_id: DocumentId) {
        self.with_driver_blocking(|driver| async move {
            if driver.is_none() {
                return;
            }
            driver
                .as_ref()
                .unwrap()
                .request_checkout(&branch_doc_id).await;
        });
    }

    fn get_change(&self, hash: ChangeHash) -> Option<&impl ChangeViewModel> {
        self.changes.get(&hash)
    }

    fn get_default_diff(&self) -> Option<impl DiffViewModel> {
        let heads_before;
        let heads_after;
        let driver = self.driver.clone();
        let branch_state = self
            .runtime
            .block_on(self.runtime.spawn(async move {
                let driver = driver.lock().await;
                if driver.is_none() {
                    return None;
                }
                driver
                    .as_ref()
                    .unwrap()
                    .get_branch_state(&driver.as_ref().unwrap().get_checked_out_ref().await?.branch)
                    .await
            }))
            .unwrap()?;

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

        // TODO (Lilith): Make synced heads work again
        heads_after = branch_state.synced_heads.clone();

        // generate the summary
        let title;
        if self.is_merge_preview_branch_active() {
            let source_name = self
                .get_branch(&branch_state.fork_info.as_ref()?.forked_from.clone())?
                .get_name();
            let target_name = self
                .get_branch(&branch_state.merge_info.as_ref()?.merge_into.clone())?
                .get_name();
            title = format!("Showing changes for {} -> {}", source_name, target_name);
        } else if self.is_revert_preview_branch_active() {
            let source_name = self
                .get_branch(&branch_state.fork_info.as_ref()?.forked_from.clone())?
                .get_name();
            // assume reverted_to is always just 1 hash
            let short_heads = &branch_state
                .revert_info
                .as_ref()?
                .reverted_to
                .first()?
                .to_string()[..7];
            title = format!(
                "Showing changes for {} reverted to {}",
                source_name, short_heads
            );
        } else {
            let source_name = self
                .get_branch(&branch_state.fork_info.as_ref()?.forked_from.clone())?
                .get_name();
            title = format!(
                "Showing changes from {} -> {}",
                source_name, branch_state.name
            );
        }

        let before = HistoryRef {
            branch: branch_state.doc_handle.document_id().clone(),
            heads: heads_before,
        };

        let after = HistoryRef {
            branch: branch_state.doc_handle.document_id().clone(),
            heads: heads_after,
        };

        Some(DiffWrapper {
            diff: self.get_cached_diff(before, after),
            title,
        })
    }

    fn get_diff(&self, selected_hash: ChangeHash) -> Option<impl DiffViewModel> {
        let change = self.changes.get(&selected_hash)?;
        if change.is_setup() {
            return None;
        }
        let heads_before;
        let heads_after = vec![change.hash];

        let history = self.get_branch_history();
        let mut prev_hash = None;
        for (i, el) in history.iter().enumerate() {
            if *el == selected_hash {
                prev_hash = history.get(i - 1).copied();
                break;
            }
        }

        let driver = self.driver.clone();
        let branch_state = self
            .runtime
            .block_on(self.runtime.spawn(async move {
                let driver = driver.lock().await;
                if driver.is_none() {
                    return None;
                }
                driver
                    .as_ref()
                    .unwrap()
                    .get_branch_state(&driver.as_ref().unwrap().get_checked_out_ref().await?.branch)
                    .await
            }))
            .unwrap()?;

        if let Some(prev_hash) = prev_hash {
            heads_before = vec![prev_hash];
        } else {
            heads_before = branch_state.fork_info.as_ref()?.forked_at.clone();
        }

        let before = HistoryRef {
            branch: branch_state.doc_handle.document_id().clone(),
            heads: heads_before,
        };

        let after = HistoryRef {
            branch: branch_state.doc_handle.document_id().clone(),
            heads: heads_after,
        };

        Some(DiffWrapper {
            diff: self.get_cached_diff(before, after),
            title: format!(
                "Showing changes from {} - {}",
                change.get_summary(),
                change.get_human_timestamp()
            ),
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
        Some(
            self.metadata
                .as_ref()?
                .merge_metadata
                .as_ref()?
                .merged_branch_id
                .clone(),
        )
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
        !self
            .state
            .doc_handle
            .with_document(|d| d.get_heads().len() == 0)
    }

    fn get_reverted_to(&self) -> Option<ChangeHash> {
        Some(
            self.state
                .revert_info
                .as_ref()?
                .reverted_to
                .first()?
                .clone(),
        )
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
