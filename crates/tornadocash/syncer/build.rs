fn main() {
    cfg_aliases::cfg_aliases! {
        native: { not(target_arch = "wasm32") },
        wasm: { target_arch = "wasm32" },
    }
}
