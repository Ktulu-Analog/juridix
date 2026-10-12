fn main() {
    println!("cargo:rerun-if-changed=prompts.yml");
    // Clé publique de signature du pack MCP, compilée dans l'application (mcp_pack.rs).
    println!("cargo:rerun-if-env-changed=JURIDIX_MCP_PACK_PUBKEY");
    tauri_build::build()
}
