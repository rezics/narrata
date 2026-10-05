//! Platform adapters for the filesystem-free authoring minter.

use std::time::{SystemTime, UNIX_EPOCH};

pub use narrata_authoring::ids::package_ids;
use narrata_nodes::{AuthoredId, Error, ExecutionId, PackageSource, Result};

fn clock() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::new("clock", "ids", "the system clock is before 1970"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| Error::new("clock", "ids", "timestamp exceeds u64"))
}
fn random(bytes: &mut [u8]) -> Result<()> {
    getrandom::fill(bytes).map_err(|e| Error::new("random", "ids", e.to_string()))
}
pub fn uuid_v7() -> Result<[u8; 16]> {
    narrata_authoring::ids::uuid_v7(&mut clock, &mut random)
}
pub fn execution_id() -> Result<ExecutionId> {
    Ok(ExecutionId::from_bytes(uuid_v7()?))
}

type NativeMinter = narrata_authoring::Minter<fn() -> Result<u64>, fn(&mut [u8]) -> Result<()>>;
pub struct Minter(NativeMinter);
impl Minter {
    pub fn new(taken: impl IntoIterator<Item = AuthoredId>) -> Self {
        Self(NativeMinter::new(taken, clock, random))
    }
}
impl std::ops::Deref for Minter {
    type Target = NativeMinter;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Minter {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
pub fn fill(package: &mut PackageSource, minter: &mut Minter) -> Result<()> {
    narrata_authoring::ids::fill(package, &mut minter.0)
}
