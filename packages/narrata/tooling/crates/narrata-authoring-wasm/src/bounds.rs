//! JavaScript numbers cross as f64, before any wasm-bindgen integer coercion.
use narrata_authoring::relations::{MAX_EXPANDED_ROWS, MAX_QUERY_ROWS};
use narrata_nodes::{Error, Result};
pub(crate) fn index(value: f64, maximum: usize) -> Result<usize> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > maximum as f64 {
        return Err(Error::new(
            "limit",
            "relations",
            "expected a bounded nonnegative integer",
        ));
    }
    Ok(value as usize)
}
pub(crate) fn page(offset: f64, limit: f64) -> Result<(usize, usize)> {
    let offset = index(offset, MAX_EXPANDED_ROWS)?;
    let limit = index(limit, MAX_QUERY_ROWS)?;
    if limit == 0 {
        return Err(Error::new(
            "limit",
            "relations",
            "page limit must be positive",
        ));
    }
    Ok((offset, limit))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_before_integer_conversion() {
        for value in [
            4294967297.0,
            1.5,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
        ] {
            assert!(page(value, 1.0).is_err());
            assert!(page(0.0, value).is_err());
            assert!(index(value, MAX_EXPANDED_ROWS).is_err());
        }
        assert_eq!(page(0.0, 1.0).ok(), Some((0, 1)));
        assert!(page(0.0, 0.0).is_err());
    }
}
