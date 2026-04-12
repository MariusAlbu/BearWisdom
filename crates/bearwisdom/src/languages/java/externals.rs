/// Runtime globals always external for Java.
pub(crate) const EXTERNALS: &[&str] = &[
    // SLF4J logging (ubiquitous)
    "Logger", "LoggerFactory",
];
