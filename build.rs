// Intègre l'icône et les infos de version dans l'exécutable Windows.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("FileDescription", "Traducteur de livres")
            .set("ProductName", "Traducteur de livres");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Icône non intégrée à l'exécutable : {e}");
        }
    }
}
