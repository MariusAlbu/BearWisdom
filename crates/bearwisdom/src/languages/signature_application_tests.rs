use super::LanguagePlugin;

fn assert_angle_application(plugin: &dyn LanguagePlugin) {
    assert_eq!(
        plugin.signature_type_application("Envelope<Alpha, Beta>"),
        (
            "Envelope".to_string(),
            vec!["Alpha".to_string(), "Beta".to_string()],
        )
    );
}

fn assert_bracket_application(plugin: &dyn LanguagePlugin) {
    assert_eq!(
        plugin.signature_type_application("Envelope[Alpha, Beta]"),
        (
            "Envelope".to_string(),
            vec!["Alpha".to_string(), "Beta".to_string()],
        )
    );
}

#[test]
fn angle_application_languages_opt_in_explicitly() {
    assert_angle_application(&super::c_lang::CLangPlugin);
    assert_angle_application(&super::csharp::CSharpPlugin);
    assert_angle_application(&super::dart::DartPlugin);
    assert_angle_application(&super::fsharp::FSharpPlugin);
    assert_angle_application(&super::groovy::GroovyPlugin);
    assert_angle_application(&super::java::JavaPlugin);
    assert_angle_application(&super::kotlin::KotlinPlugin);
    assert_angle_application(&super::pascal::PascalPlugin);
    assert_angle_application(&super::rust_lang::RustLangPlugin);
    assert_angle_application(&super::swift::SwiftPlugin);
    assert_angle_application(&super::typescript::TypeScriptPlugin);
}

#[test]
fn bracket_application_languages_opt_in_explicitly() {
    assert_bracket_application(&super::go::GoPlugin);
    assert_bracket_application(&super::nim::NimPlugin);
    assert_bracket_application(&super::python::PythonPlugin);
    assert_bracket_application(&super::scala::ScalaPlugin);
}

#[test]
fn non_owning_language_does_not_inherit_type_application_syntax() {
    assert_eq!(
        super::ada::AdaPlugin.signature_type_application("Envelope<Alpha>"),
        ("Envelope<Alpha>".to_string(), Vec::new())
    );
}

#[test]
fn borrowed_type_head_remains_plugin_owned() {
    assert_eq!(
        super::typescript::TypeScriptPlugin.signature_type_head("Envelope<Alpha>"),
        "Envelope"
    );
    assert_eq!(
        super::ada::AdaPlugin.signature_type_head("Envelope<Alpha>"),
        "Envelope<Alpha>"
    );
}
