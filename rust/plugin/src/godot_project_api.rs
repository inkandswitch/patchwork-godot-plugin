use automerge::ChangeHash;
use automerge_repo::DocumentId;

pub enum SyncStatus {
    /// The server is disconnected, but we have no idea if we have extra changes.
    Unknown,
    /// The server is disconnected, and we know how many changes we haven't pushed.
    Disconnected(usize),
    /// The server is currently syncing our changes.
    Syncing,
    /// The server is up to date with our changes.
    UpToDate
}

/// Defines the surface for the UI layer interacting with the GodotProject core logic.
pub trait GodotProjectViewModel {
    // PROJECT MANAGEMENT
    fn clear_project(&mut self);
    fn get_user_name(&self) -> String;
    fn set_user_name(&self, name: String);
    fn get_sync_status(&self) -> SyncStatus;
    fn print_sync_debug(&self);
    
    // MERGE & REVERT
    fn can_create_merge_preview_branch(&self) -> bool;
    fn create_merge_preview_branch(&mut self);

    fn can_create_revert_preview_branch(&self, head: ChangeHash) -> bool;
    fn create_revert_preview_branch(&mut self, head: ChangeHash);

    fn preview_branch_active(&self) -> bool;
    fn confirm_preview_branch(&mut self);
    fn discard_preview_branch(&mut self);

    // CHANGES
    fn get_branch_history(&self) -> Vec<ChangeHash>;
    // todo: do we want these on a separate struct as properties?
    fn get_change_username(&self, hash: ChangeHash) -> String;
    fn is_change_synced(&self, hash: ChangeHash) -> bool;
    fn get_change_summary(&self, hash: ChangeHash) -> String;
    fn is_change_merge(&self, hash: ChangeHash) -> bool;
    fn is_change_setup(&self, hash: ChangeHash) -> bool;
    fn get_change_exact_timestamp(&self, hash: ChangeHash) -> String;
    fn get_change_human_timestamp(&self, hash: ChangeHash) -> String;
    fn get_change_merge_id(&self, hash: ChangeHash) -> Option<DocumentId>;
    
    // TODO: keep adding API here
}
