//! coneko-term の本体。ブラウザーに依存しない。

pub mod screen;

/// このライブラリの版を返す。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_cargo_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
