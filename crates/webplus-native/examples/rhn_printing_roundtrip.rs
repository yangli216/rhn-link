//! Loads the RHN printing adapter through the generic DLL mapping and
//! exercises only calls that cannot create a print job: printer enumeration,
//! a digest mismatch and an unknown printer.

#[cfg(windows)]
fn main() {
    use std::fs;

    use serde_json::{json, Map, Value};
    use webplus_native::NativePlugin;
    use webplus_plugin_config::PluginManifest;
    use webplus_protocol::{InvokeRequest, PluginArchitecture};

    // Synthetic document: never reaches a spooler because every printPdf call
    // below fails before the printer is opened.
    const DOCUMENT_BASE64: &str = "JVBERi0xLjQKJXJobi1saW5rIHByaW50aW5nIHJvdW5kdHJpcAo=";
    const DOCUMENT_SHA256: &str =
        "e77b3d1a467cb238f0b60213493b98c744b3f31d9ae85b13a72d12dfc393ff7d";
    const OTHER_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    let mut arguments = std::env::args_os().skip(1);
    let dll = arguments
        .next()
        .expect("usage: rhn_printing_roundtrip <rhn_printing_plugin.dll> <api.json>");
    let api = arguments
        .next()
        .expect("usage: rhn_printing_roundtrip <rhn_printing_plugin.dll> <api.json>");
    let plugin_dir = tempfile::tempdir().expect("failed to create printing plugin directory");
    fs::create_dir(plugin_dir.path().join("bin")).expect("failed to create bin directory");
    fs::copy(dll, plugin_dir.path().join("bin/rhn_printing_plugin.dll"))
        .expect("failed to copy printing DLL");
    fs::copy(api, plugin_dir.path().join("api.json")).expect("failed to copy printing api.json");

    let manifest = PluginManifest::load("rhn-printing", plugin_dir.path())
        .expect("failed to load RHN printing manifest");
    let mut plugin = NativePlugin::new(manifest);
    let architecture = if cfg!(target_arch = "x86") {
        PluginArchitecture::X86
    } else {
        PluginArchitecture::X64
    };
    assert_eq!(plugin.preflight(architecture).unwrap(), 1);

    let invoke = |plugin: &mut NativePlugin, method: &str, parameters: Map<String, Value>| {
        let response = plugin.invoke(&InvokeRequest {
            service_id: "rhn.printing".into(),
            method: method.into(),
            parameters,
        });
        assert_eq!(response.res_code, 0, "{method}: {response:?}");
        let data = response.res_data;
        let result: Value = serde_json::from_str(data["result"].as_str().unwrap()).unwrap();
        (data["ReturnValue"].as_u64().unwrap(), result)
    };

    let (status, listed) = invoke(&mut plugin, "listPrinters", Map::new());
    assert_eq!(status, 0, "{listed}");
    let printers = listed["printers"].as_array().unwrap();
    assert!(printers.iter().all(|printer| {
        printer["name"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
            && printer["isDefault"].is_boolean()
    }));
    assert!(
        printers
            .iter()
            .filter(|printer| printer["isDefault"] == true)
            .count()
            <= 1
    );

    let print = |sha256: &str, printer: &str| {
        Map::from_iter([
            ("printerName".into(), json!(printer)),
            ("documentBase64".into(), json!(DOCUMENT_BASE64)),
            ("sha256".into(), json!(sha256)),
            ("copies".into(), json!(1)),
            ("jobName".into(), json!("rhn-link-ci")),
        ])
    };
    let missing_printer = "rhn-link-ci-printer-that-does-not-exist";
    let (status, mismatch) = invoke(
        &mut plugin,
        "printPdf",
        print(OTHER_SHA256, missing_printer),
    );
    assert_eq!(
        (status, mismatch),
        (1002, json!({ "error": "document-digest-mismatch" }))
    );
    let (status, unknown) = invoke(
        &mut plugin,
        "printPdf",
        print(DOCUMENT_SHA256, missing_printer),
    );
    assert_eq!(
        (status, unknown),
        (1004, json!({ "error": "printer-not-found" }))
    );

    println!(
        "RHN printing plugin round-trip succeeded with {} printer(s)",
        printers.len()
    );
}

#[cfg(not(windows))]
fn main() {
    eprintln!("rhn_printing_roundtrip must run on Windows");
    std::process::exit(2);
}
