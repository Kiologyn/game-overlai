use std::path::Path;

fn main() {
    // The UI is embedded from ../web/dist (rust-embed). Make sure the folder exists so a fresh
    // checkout compiles; the placeholder tells you to build the frontend.
    let dist = Path::new("../web/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).expect("create web/dist");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=utf-8><body style=\"font:16px system-ui;color:#ddd;background:#16161c;padding:24px\">\
             UI is not built. Run <code>npm run build</code> in <code>web/</code> \
             (or start <code>npm run dev</code> and set <code>OVERLAI_UI_DEV=http://localhost:5173</code>).",
        )
        .expect("write placeholder");
        println!("cargo:warning=web/dist was missing: wrote a placeholder page (run `npm run build` in web/)");
    }
    println!("cargo:rerun-if-changed=../web/dist");
    tauri_build::build()
}
