fn main() {
    let cert_path = std::path::Path::new("hub_cert.pem");

    if !cert_path.exists() {
        println!("cargo:warning=================================");
        println!("cargo:warning=hub_cert.pem not found!");
        println!("cargo:warning=HTTPS will not work.");
        println!("cargo:warning=Copy hub_tls_cert.pem to probe/hub_cert.pem");
        println!("cargo:warning=================================");
    }

    // Trigger rebuild if the certificate changes
    println!("cargo:rerun-if-changed=hub_cert.pem");
}
