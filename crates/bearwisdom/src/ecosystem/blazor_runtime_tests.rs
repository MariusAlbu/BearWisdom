use super::*;

#[test]
fn synth_emits_blazor_interface_with_runtime_methods() {
    let file = synthesize_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();

    // Canonical circuit-lifecycle methods seen in the Microsoft
    // ReconnectModal.razor.js template.
    for required in &["reconnect", "resumeCircuit", "pauseCircuit", "start"] {
        assert!(
            names.contains(required),
            "Blazor runtime synthetic must declare {required}; got: {names:?}"
        );
    }

    // Top-level `Blazor` carrier — both an interface and a variable so
    // chain-receiver and bare-global refs both resolve.
    let blazor_syms: Vec<&ExtractedSymbol> = file
        .symbols
        .iter()
        .filter(|s| s.name == "Blazor")
        .collect();
    assert!(
        blazor_syms.iter().any(|s| s.kind == SymbolKind::Interface),
        "must emit `Blazor` as an interface for chain-walker receiver typing"
    );
    assert!(
        blazor_syms.iter().any(|s| s.kind == SymbolKind::Variable),
        "must emit `Blazor` as a variable so bare global refs resolve"
    );
}

#[test]
fn method_qnames_are_dotted_under_blazor() {
    let file = synthesize_file();
    let has_reconnect = file
        .symbols
        .iter()
        .any(|s| s.qualified_name == "Blazor.reconnect" && s.kind == SymbolKind::Property);
    assert!(
        has_reconnect,
        "Blazor.reconnect must be a dotted property qname so chain walker finds it"
    );
}

#[test]
fn activation_language_set_covers_razor_and_csharp() {
    let eco = BlazorRuntimeEcosystem;
    match eco.activation() {
        EcosystemActivation::Any(options) => {
            let mut saw_razor = false;
            let mut saw_csharp = false;
            for opt in options {
                if let EcosystemActivation::LanguagePresent(lang) = opt {
                    match *lang {
                        "razor" => saw_razor = true,
                        "csharp" => saw_csharp = true,
                        _ => {}
                    }
                }
            }
            assert!(saw_razor && saw_csharp,
                "Blazor runtime must activate on both razor and csharp — Blazor Server projects \
                 may ship .razor.js without any .razor file in the indexed slice");
        }
        _ => panic!("expected Any(…) activation for blazor-runtime"),
    }
}
