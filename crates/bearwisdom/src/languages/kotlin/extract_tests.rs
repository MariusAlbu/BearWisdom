    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn package_header_emits_namespace_with_dotted_qname() {
        let r = extract::extract("package com.foo.bar\n\nclass X");
        let ns = r
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Namespace)
            .expect("namespace");
        assert_eq!(ns.qualified_name, "com.foo.bar");
        assert_eq!(ns.name, "bar");
    }

    #[test]
    fn package_header_prefixes_top_level_class_qname() {
        let r = extract::extract("package com.foo\n\nclass Bar");
        let bar = r.symbols.iter().find(|s| s.name == "Bar").expect("Bar");
        assert_eq!(bar.qualified_name, "com.foo.Bar");
        assert_eq!(bar.scope_path.as_deref(), Some("com.foo"));
    }

    #[test]
    fn package_header_prefixes_nested_class_qname() {
        let src = "package com.foo\n\nclass Outer {\n  class Inner\n}";
        let r = extract::extract(src);
        let inner = r.symbols.iter().find(|s| s.name == "Inner").expect("Inner");
        assert_eq!(inner.qualified_name, "com.foo.Outer.Inner");
    }

    #[test]
    fn package_header_prefixes_top_level_function() {
        let r = extract::extract("package com.foo\n\nfun greet(): String = \"hi\"");
        let fn_sym = r.symbols.iter().find(|s| s.name == "greet").expect("greet");
        assert_eq!(fn_sym.qualified_name, "com.foo.greet");
    }

    #[test]
    fn package_header_prefixes_constructor_promoted_property() {
        let r = extract::extract("package com.foo\n\nclass Box(val item: String)");
        let item = r.symbols.iter().find(|s| s.name == "item").expect("item");
        assert_eq!(item.qualified_name, "com.foo.Box.item");
    }

    #[test]
    fn no_package_header_leaves_qname_unprefixed() {
        let r = extract::extract("class Standalone");
        let s = r.symbols.iter().find(|s| s.name == "Standalone").expect("Standalone");
        assert_eq!(s.qualified_name, "Standalone");
    }

    #[test]
    fn extracts_class_with_method() {
        let src = r#"
class Animal(val name: String) {
    fun speak(): String {
        return "..."
    }
}
"#;
        let r = extract::extract(src);
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
        assert_eq!(cls.kind, SymbolKind::Class);

        let method = r.symbols.iter().find(|s| s.name == "speak").expect("speak");
        assert_eq!(method.kind, SymbolKind::Method);
    }

    #[test]
    fn extracts_enum_class() {
        let src = r#"
enum class Direction {
    NORTH,
    SOUTH,
    EAST,
    WEST
}
"#;
        let r = extract::extract(src);
        let en = r.symbols.iter().find(|s| s.name == "Direction").expect("Direction");
        assert_eq!(en.kind, SymbolKind::Enum);
        // Enum members depend on grammar version; at least the enum itself must be present.
        assert!(!r.symbols.is_empty());
    }

    #[test]
    fn companion_object_extracted_as_class() {
        let src = r#"
class Config {
    companion object {
        val DEFAULT_TIMEOUT = 30
        fun create(): Config = Config()
    }
}
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "Companion" && s.kind == SymbolKind::Class),
            "Companion not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        // create() should be extracted as a member inside the companion.
        assert!(r.symbols.iter().any(|s| s.name == "create"));
    }

    #[test]
    fn as_expression_emits_type_ref() {
        let src = r#"
fun cast(x: Any): String {
    return x as String
}
"#;
        let r = extract::extract(src);
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
            "TypeRef for String not found; refs: {:?}",
            r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn primary_constructor_promoted_params_extracted() {
        let src = r#"
class Point(val x: Double, val y: Double)
"#;
        let r = extract::extract(src);
        // val x and val y become Property symbols.
        assert!(
            r.symbols.iter().any(|s| s.name == "x" && s.kind == SymbolKind::Property),
            "x property not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(r.symbols.iter().any(|s| s.name == "y" && s.kind == SymbolKind::Property));
        // TypeRefs for Double emitted.
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "Double" && rf.kind == EdgeKind::TypeRef),
            "TypeRef for Double not found; refs: {:?}",
            r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn interface_and_class_extracted() {
        let src = r#"
interface Drawable {
    fun draw()
}

class Circle : Drawable {
    override fun draw() {}
}
"#;
        let r = extract::extract(src);
        // Kotlin grammar may emit interface_declaration or class_declaration for interfaces
        assert!(
            r.symbols.iter().any(|s| s.name == "Drawable"),
            "Drawable not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(r.symbols.iter().any(|s| s.name == "Circle" && s.kind == SymbolKind::Class));
    }

    #[test]
    fn annotated_local_var_in_function_body() {
        // @Annotation before local val/var in function body should emit TypeRef for annotation
        let src = r#"
fun foo() {
    @Suppress("UNCHECKED_CAST")
    val x: List<String> = listOf()
}
"#;
        let r = extract::extract(src);
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "Suppress"),
            "expected Suppress annotation ref; refs: {:?}",
            r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn init_block_property_extracted() {
        // Properties inside init blocks should produce Property symbols
        let src = r#"
class Config {
    val x: Int
    init {
        val temp: TempType = TempType()
        x = temp.value
    }
}
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "temp"),
            "expected 'temp' in init block; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn extension_property_extracted() {
        // Extension properties on receiver types should produce Property symbols
        let src = r#"
val String.reversed: String get() = this.reversed()
var Int.doubled: Int get() = this * 2
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "reversed"),
            "extension property 'reversed' not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lambda_body_property_extracted() {
        // Properties inside lambda bodies (run, let, apply, etc.) should produce symbols
        let src = r#"
fun setup() {
    run {
        val config: Config = Config()
        val timeout = 30
    }
    listOf(1, 2, 3).forEach { item ->
        val doubled: Int = item * 2
    }
}
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "config"),
            "expected 'config' in run block; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn annotated_property_emits_inject_ref() {
        // @Inject annotation on property should emit TypeRef
        let src = r#"
@Component
class Service {
    @Inject
    lateinit var repo: Repository
}
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "repo"),
            "expected 'repo'; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "Inject"),
            "expected Inject annotation ref; refs: {:?}",
            r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn function_param_name_not_emitted_as_calls_ref() {
        // `modifier = modifier` in a named argument must not emit `modifier` as a Calls ref.
        // `modifier` is a function parameter — a local — not a callable symbol.
        let src = r#"
fun BottomNavigationBar(
    navController: NavController,
    modifier: Modifier = Modifier,
) {
    NavigationBar(
        modifier = modifier,
    ) { }
}
"#;
        let r = extract::extract(src);
        let calls_refs: Vec<_> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.clone())
            .collect();
        assert!(
            !calls_refs.contains(&"modifier".to_string()),
            "modifier (function param) must not appear as Calls ref; refs: {calls_refs:?}"
        );
    }

    #[test]
    fn fq_chain_intermediate_segments_not_emitted() {
        // `com.example.foo.SomeClass.doSomething()` — only `doSomething` (Calls)
        // and `SomeClass` (TypeRef from emit_chain_type_ref) should be emitted.
        // Package segments like `com`, `example`, `foo` must not appear as refs.
        let src = r#"
fun test() {
    com.example.foo.SomeClass.doSomething()
}
"#;
        let r = extract::extract(src);
        let all_targets: Vec<_> = r.refs.iter()
            .map(|rf| (rf.target_name.clone(), rf.kind))
            .collect();
        assert!(
            !all_targets.iter().any(|(t, _)| t == "com"),
            "`com` must not be emitted as a ref; refs with kinds: {all_targets:?}"
        );
        assert!(
            !all_targets.iter().any(|(t, _)| t == "example"),
            "`example` must not be emitted as a ref; refs with kinds: {all_targets:?}"
        );
    }

    #[test]
    fn local_val_not_emitted_as_calls_ref() {
        // `val response = httpClient.get(url)` — subsequent uses of `response`
        // (e.g. `response.body()`) must not emit `response` as a bare Calls ref.
        let src = r#"
fun fetch(url: String) {
    val response = httpClient.get(url)
    val body = response.body()
    println(body)
}
"#;
        let r = extract::extract(src);
        let calls_refs: Vec<_> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.clone())
            .collect();
        assert!(
            !calls_refs.contains(&"response".to_string()),
            "local `response` must not appear as bare Calls ref; refs: {calls_refs:?}"
        );
    }
