use narrata_core::ExecutionId;
pub use narrata_history::{BranchId, NameError, RefKey, RefName, RefNamespace};

narrata_kernel::authored_id!(TimelineOperationId, "timeline-op:");
narrata_kernel::authored_id!(LeaseId, "lease:");

pub type ArchiveRefName = RefName;

/// The head of `branch` among the branches of Execution `timeline`: the Ref
/// `branches/<timeline hex>/<branch hex>`.
pub fn timeline_branch(timeline: ExecutionId, branch: BranchId) -> RefKey {
    RefKey::branch(RefName::hex(timeline.as_bytes()), branch)
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CatalogRefKey {
    timeline: ExecutionId,
}

impl CatalogRefKey {
    pub const fn new(timeline: ExecutionId) -> Self {
        Self { timeline }
    }

    pub const fn timeline(&self) -> ExecutionId {
        self.timeline
    }

    pub fn storage_key(&self) -> String {
        format!(
            "catalogs/{}/complete",
            hex::encode(self.timeline.as_bytes())
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TimelineArchiveRefKey {
    timeline: ExecutionId,
    name: RefName,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CompoundSaveRefKey {
    owner: RefName,
    slot: RefName,
}

impl CompoundSaveRefKey {
    pub const fn new(owner: RefName, slot: RefName) -> Self {
        Self { owner, slot }
    }

    pub fn owner(&self) -> &RefName {
        &self.owner
    }

    pub fn slot(&self) -> &RefName {
        &self.slot
    }

    pub fn storage_key(&self) -> String {
        format!("compound-saves/{}/{}", self.owner, self.slot)
    }
}

impl TimelineArchiveRefKey {
    pub const fn new(timeline: ExecutionId, name: RefName) -> Self {
        Self { timeline, name }
    }

    pub const fn timeline(&self) -> ExecutionId {
        self.timeline
    }

    pub fn name(&self) -> &RefName {
        &self.name
    }

    pub fn storage_key(&self) -> String {
        format!(
            "archives/{}/{}",
            hex::encode(self.timeline.as_bytes()),
            self.name
        )
    }
}
