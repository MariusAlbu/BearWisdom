# Connector Migration — per-language flattening status

Tracking document for Task #19 in `PIPELINE_REFACTOR_PLAN.md`: move each
connector's detection logic from the `connectors/registry.rs` DB-query path
into its owning language plugin's `extract_connection_points` trait method.

## Infrastructure (done)

- `LanguagePlugin::extract_connection_points(source, file_path, lang_id) -> Vec<crate::types::ConnectionPoint>` — per-file hook invoked during parse. Default impl returns empty.
- `ParsedFile::connection_points: Vec<ConnectionPoint>` — collection slot on every parsed file.
- `connectors::from_plugins::collect_plugin_connection_points(parsed, file_id_map, symbol_id_map) -> Vec<connectors::types::ConnectionPoint>` — bridge that joins plugin-emitted abstract points with DB IDs, translating `ConnectionKind` → `Protocol` and `ConnectionRole` → `FlowDirection`.
- `ConnectorRegistry::run_with_plugin_points(conn, project_root, ctx, plugin_points)` — registry entry that ingests pre-collected points alongside legacy `Connector::extract` output; dedupes on `(file_id, line, protocol, direction, key, method)` so mid-migration duplication from both sources is harmless.
- `indexer/full.rs` calls the bridge + `run_with_plugin_points` instead of `run`.

Net effect: when a plugin emits connection points, they flow into the matcher exactly like legacy-extracted ones. An individual connector migration is:

1. Move its detection logic from `Connector::extract` (which reads from DB + disk) into a free function taking `source: &str` and returning `Vec<crate::types::ConnectionPoint>`.
2. Add `LanguagePlugin::extract_connection_points` on the owning plugin that calls the free function (usually via a per-language composer that fans out to every migrated connector on that plugin).
3. Make `Connector::extract` return `Ok(Vec::new())` so the point isn't emitted twice. Leave `detect()` firing so the registry still considers the protocol live.
4. Keep tests for the detection function (source-string in, ConnectionPoint out).

For REST connectors that detect both starts (client calls — source-scan) and stops (routes — populated by the parser into the `routes` table during extract), the starts half migrates and the stops half stays in the legacy `Connector::extract` path.

## Migrated connectors

Grouped by plugin. Each composer is invoked by `LanguagePlugin::extract_connection_points`.

### GraphQL (`GraphQlPlugin::extract_connection_points`)
- `graphql_schema_starts` — scans `.graphql`/`.gql` for `type Query/Mutation/Subscription`.

### Proto (`ProtoPlugin::extract_connection_points`)
- `proto_grpc_starts` — scans `.proto` for `service Foo { rpc ... }`.

### Svelte (`SveltePlugin::extract_connection_points`)
- `svelte_graphql` — embedded schema blocks + resolver maps.

### Vue (`VuePlugin::extract_connection_points`)
- `vue_graphql` — embedded schema blocks + resolver maps.

### TypeScript (`TypeScriptPlugin::extract_connection_points` via `extract_typescript_connection_points`)
- `typescript_graphql` — SDL blocks, resolver maps, type-graphql decorators.
- `tauri_ipc_ts` — `invoke("cmd")` (Start) + `listen("event")` (Stop).
- `electron_ipc` — `ipcMain.handle/on` (Stop) + `ipcRenderer.invoke/send` (Start).
- `typescript_rest` **starts** — `fetch('/url')` + `axios.get/post/...`.
- `typescript_mq` — kafkajs, amqplib, bullmq.

### Rust (`RustLangPlugin::extract_connection_points` via `extract_rust_connection_points`)
- `tauri_ipc_rust` — `#[tauri::command]` (Stop) + `.emit("name")` (Start).
- `rust_rest` **starts** — `reqwest::get/post/...`.
- `rust_mq` — rdkafka, lapin, async-nats.

### Python (`PythonPlugin::extract_connection_points` via `extract_python_connection_points`)
- `python_graphql_resolvers` — ariadne + strawberry decorators.
- `python_rest` **starts** — `requests.*` + `httpx.*`.
- `python_mq` — celery, kafka-python, pika.

### Ruby (`RubyPlugin::extract_connection_points`)
- `ruby_graphql` — `field :name` declarations + `def resolve`/`def field_name` in resolver classes.

### Go (`GoPlugin::extract_connection_points` via `extract_go_connection_points`)
- `go_rest` **starts** — `http.Get/Post` + `http.NewRequest("METHOD", ...)`.
- `go_mq` — sarama `Topic:`, amqp091 `.Publish/.Consume`, nats `.Publish/.Subscribe`.

### Java (`JavaPlugin::extract_connection_points` via `extract_java_connection_points`)
- `java_rest` **starts** — `HttpClient/RestTemplate/WebClient` method calls.
- `java_mq` — kafkaTemplate, ProducerRecord, rabbitTemplate, `@KafkaListener`, `@RabbitListener`.

### Kotlin (`KotlinPlugin::extract_connection_points` via `extract_kotlin_connection_points`)
- `kotlin_rest` **starts** — Retrofit `@GET("/path")`, OkHttp `.url(...)`, Ktor client.
- `kotlin_mq` — kafkaTemplate, rabbitTemplate, `@KafkaListener`, `@RabbitListener`.

### Swift (`SwiftPlugin::extract_connection_points` via `extract_swift_connection_points`)
- `swift_rest` **starts** — URLSession (`URL(string:)`), Alamofire, URLRequest.

### Dart (`DartPlugin::extract_connection_points` via `extract_dart_connection_points`)
- `dart_rest` **starts** — http package, Dio, Chopper annotations.

### PHP (`PhpPlugin::extract_connection_points` via `extract_php_connection_points`)
- `php_rest` **starts** — Guzzle / Laravel Http facade.

### C# (`CSharpPlugin::extract_connection_points` inline)
- `csharp_mq` — MassTransit/NServiceBus `Publish/Send`, Service Bus trigger, RabbitMQ/Kafka.

### Groovy (`GroovyPlugin::extract_connection_points` via `extract_groovy_connection_points`)
- `groovy_spring_routes` — `@GetMapping/@PostMapping/@RequestMapping` on Spring controllers.

## DB-lookup connectors — all flattened into `resolve_connection_points`

The `LanguagePlugin::resolve_connection_points(db, project_root, ctx)` post-parse
hook is the home for connectors that need cross-file joins, inheritance
lookups, or DI-container resolution. Every plugin with such a connector
overrides this method and drives the legacy `Connector` via
`crate::languages::drive_connector`, which preserves the existing regex +
SQL query bodies while moving invocation ownership from the registry to the
plugin.

| Plugin | Connectors migrated to `resolve_connection_points` |
|---|---|
| C# | `dotnet_di`, `event_bus`, `csharp_grpc`, `csharp_graphql_resolvers`, `csharp_rest` |
| F# | `fsharp_di` |
| VB.NET | `vbnet_di` |
| Angular | `angular_di`, `angular_rest` |
| Java | `spring_routes`, `spring_di`, `java_rest` (stops), `java_grpc_stops` |
| Kotlin | `kotlin_grpc_stops`, `kotlin_rest` (stops) |
| TypeScript | `nestjs_routes`, `nextjs_routes`, `typescript_rest` (stops) |
| Python | `django_routes`, `fastapi_routes`, `python_rest` (stops), `python_grpc_stops`, `python_graphql_resolvers` (Graphene half) |
| Elixir | `phoenix_routes` |
| Ruby | `rails_routes`, `ruby_rest` (stops) |
| PHP | `laravel_routes`, `php_rest` (stops) |
| Go | `go_route`, `go_rest` (stops), `go_grpc_stops` |
| Rust | `rust_rest` (stops), `rust_grpc_stops` |
| Swift | `swift_rest` (stops) |
| Dart | `dart_rest` (stops) |
| Groovy | (neutered — now source-scan-only via `extract_connection_points`) |

Every `fn connectors() -> Vec<Box<dyn Connector>>` now returns `vec![]`. All
connector work flows through `extract_connection_points` (parse-time source
scan) or `resolve_connection_points` (post-parse DB-join hook). The legacy
registry-owned `Connector::extract` path has no callers.

## Status summary

- **All source-scan-only connectors** → migrated into plugin
  `extract_connection_points`.
- **All DB-lookup connectors** → migrated into plugin
  `resolve_connection_points`.
- **All `LanguagePlugin::connectors()` return `vec![]`** — registry is purely
  a matcher-driver over plugin-emitted points now.
- Plugin-emitted points and DB-join hook points both flow through the
  registry's `(file_id, line, protocol, direction, key, method)` dedupe.

## Recipe for reference

```rust
// In languages/{lang}/connectors.rs

pub fn extract_{lang}_connection_points(source: &str, file_path: &str)
    -> Vec<crate::types::ConnectionPoint>
{
    let mut out = Vec::new();
    extract_{domain}_src(source, file_path, &mut out);  // one per migrated connector
    // ...
    out
}

pub fn extract_{domain}_src(source: &str, file_path: &str, out: &mut Vec<AbstractPoint>) {
    // 1. Fast-path return if no marker text in source.
    if !source.contains("indicator_substring") { return; }

    // 2. Build regexes (fine at parse time — microsecond compilation cost).
    let re = regex::Regex::new(r"...").unwrap();

    // 3. Walk lines; push ConnectionPoints with the right ConnectionKind / Role.
    for (line_idx, line) in source.lines().enumerate() {
        // ...
        out.push(AbstractPoint {
            kind: ConnectionKind::Rest,      // or Grpc / GraphQL / Di / Ipc / Event / MessageQueue / Route
            role: ConnectionRole::Stop,      // or Start
            key: route_or_name,
            line: (line_idx + 1) as u32,
            col: 1,
            symbol_qname: String::new(),     // let bridge lookup handle it if needed
            meta,                             // {"method": "GET", "framework": "..."} — known keys lifted into DB columns
        });
    }
}

// In languages/{lang}/mod.rs
impl LanguagePlugin for {Lang}Plugin {
    // ...
    fn extract_connection_points(&self, source: &str, file_path: &str, _lang_id: &str)
        -> Vec<crate::types::ConnectionPoint>
    {
        connectors::extract_{lang}_connection_points(source, file_path)
    }
}

// Neuter the legacy Connector::extract:
impl Connector for {Lang}{Domain}Connector {
    // descriptor / detect unchanged
    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        Ok(Vec::new())
    }
}
```
