//! coneko-term を WebAssembly にするための wasm-bindgen の層。中身は `coneko-term` に置く。

use wasm_bindgen::prelude::*;

/// このライブラリの版を返す。
#[wasm_bindgen]
pub fn version() -> String {
    coneko_term::version().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_core() {
        assert_eq!(super::version(), coneko_term::version());
    }
}
