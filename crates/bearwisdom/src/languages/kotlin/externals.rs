/// Runtime globals always external for Kotlin.
pub(crate) const EXTERNALS: &[&str] = &[
    // SLF4J logging (ubiquitous across JVM)
    "Logger", "LoggerFactory",
];
