narrata_kernel::derived_id!(ObjectId, "object:");
narrata_kernel::derived_id!(ArtifactId, "artifact:");
narrata_kernel::authored_id!(BranchId, "branch:");

/// Content identity of an object: kind, schema and canonical payload (ADR 0002).
pub fn object_id(kind: u16, schema: u16, canonical_payload: &[u8]) -> ObjectId {
    ObjectId::from_bytes(narrata_kernel::codec::object_id(
        kind,
        schema,
        canonical_payload,
    ))
}
