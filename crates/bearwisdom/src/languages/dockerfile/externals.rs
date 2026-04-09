use std::collections::HashSet;

/// Well-known Docker base images that are always external (from Docker Hub or
/// public registries). These appear in `FROM <image>` instructions.
pub(crate) const EXTERNALS: &[&str] = &[
    // ── Linux distros ─────────────────────────────────────────────────────────
    "ubuntu",
    "debian",
    "alpine",
    "fedora",
    "centos",
    "amazonlinux",
    "oraclelinux",
    "rockylinux",
    "almalinux",
    "archlinux",
    "opensuse/leap",
    "opensuse/tumbleweed",
    // ── Language runtimes ─────────────────────────────────────────────────────
    "node",
    "python",
    "golang",
    "rust",
    "ruby",
    "php",
    "perl",
    "openjdk",
    "eclipse-temurin",
    "amazoncorretto",
    "scala",
    "clojure",
    "swift",
    "elixir",
    "erlang",
    "haskell",
    "r-base",
    "julia",
    // ── .NET / Microsoft ─────────────────────────────────────────────────────
    "mcr.microsoft.com/dotnet/aspnet",
    "mcr.microsoft.com/dotnet/sdk",
    "mcr.microsoft.com/dotnet/runtime",
    "mcr.microsoft.com/dotnet/runtime-deps",
    "mcr.microsoft.com/dotnet/monitor",
    // ── Web servers ──────────────────────────────────────────────────────────
    "nginx",
    "httpd",
    "caddy",
    "traefik",
    "haproxy",
    // ── Databases ────────────────────────────────────────────────────────────
    "postgres",
    "mysql",
    "mariadb",
    "mongo",
    "redis",
    "elasticsearch",
    "opensearch",
    "cassandra",
    "couchdb",
    "neo4j",
    "influxdb",
    "timescaledb",
    "clickhouse/clickhouse-server",
    "bitnami/postgresql",
    "bitnami/mysql",
    // ── Messaging / streaming ─────────────────────────────────────────────────
    "rabbitmq",
    "apache/kafka",
    "confluentinc/cp-kafka",
    "bitnami/kafka",
    "apache/activemq",
    "nats",
    // ── Build / CI tools ─────────────────────────────────────────────────────
    "gradle",
    "maven",
    "alpine/git",
    "docker",
    "docker/compose",
    // ── Special / minimal ────────────────────────────────────────────────────
    "scratch",
    "busybox",
    "gcr.io/distroless/base",
    "gcr.io/distroless/java",
    "gcr.io/distroless/python3",
    "cgr.dev/chainguard/static",
    // ── Cloud provider images ─────────────────────────────────────────────────
    "amazon/aws-lambda-python",
    "amazon/aws-lambda-nodejs",
    "amazon/aws-lambda-java",
    "amazon/aws-lambda-go",
    "public.ecr.aws/lambda/python",
    "public.ecr.aws/lambda/nodejs",
];

/// Dependency-gated framework globals for Dockerfiles.
/// Dockerfiles have no dependency manifest; this is a no-op.
pub(crate) fn framework_globals(_deps: &HashSet<String>) -> Vec<&'static str> {
    Vec::new()
}
