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

## Remains on legacy DB path

The registry path is still the authoritative source for connectors that need
symbol-table joins, inheritance lookups, or DI-container resolution — none of
which is available at parse time. Their `Connector::extract` stays live; the
plugin's `extract_connection_points` returns nothing for them.

### Need class/method joins or inheritance walks

- `csharp_graphql_resolvers` — Hot Chocolate `[Query]` on classes + methods.
- `csharp_rest` — route-handler join + client call starts.
- `csharp_grpc` — class-based service/client detection.
- `dotnet_di` / `fsharp_di` / `vbnet_di` — `AddScoped/Transient/Singleton` resolving `IFoo`/`Foo`.
- `event_bus` — MediatR `IRequest/IRequestHandler`, MassTransit `IConsumer<T>`.
- `angular_di` / `angular_rest` — class-decorator + providedIn analysis; HttpClient joining to injected services.
- `spring_di` / `spring_routes` — bean discovery spans files.
- `nestjs_routes` — `@Controller` on class + `@Get('/path')` on method.
- `python_graphql_resolvers` — Graphene `resolve_*` method detection (only the Graphene half; ariadne + strawberry are migrated).
- `python_grpc_stops` — `*Servicer` inheritance + method enumeration.
- `java_grpc_stops` — `*ImplBase` class extends + method walk.
- `kotlin_grpc_stops` — `*CoroutineImplBase` / `*ImplBase` class extends + method walk.
- `go_grpc_stops` — struct implements `*Server` + method walk.
- `rust_grpc_stops` — `impl *Server for` + method walk in file (uses DB symbol query for methods).

### Filesystem-map connectors

- `nextjs_routes` — filesystem-to-route mapping (pages/app router).
- `django_routes` / `fastapi_routes` — class-based views + APIView subclasses.
- `phoenix_routes` / `rails_routes` / `laravel_routes` — scope stacking across files.
- `go_route` (gin/echo/chi declarations + DB handler function lookup).

### Stop-side of REST (kept on DB)

Every `{lang}_rest` connector listed in "migrated" above has its **starts**
flattened; its **stops** continue to read from the `routes` table, which is
populated by the parser's route extraction during indexing. That is the one
remaining DB read in each of those extract methods.

## Status summary

- **All source-scan-only connectors listed in the prior version of this doc are now flattened.**
- Plugin-emitted points flow through the matcher alongside legacy DB-query output. Partial migrations coexist safely via the registry's `(file_id, line, protocol, direction, key, method)` dedupe.
- Remaining DB-only connectors cannot be flattened without either (a) a `LanguagePlugin::resolve_connection_points(points, db_handle)` post-parse hook that can see the fully-populated symbol and inheritance graph, or (b) rewriting each to pre-compute its joins in-memory at parse time. That's a separate design task.

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
