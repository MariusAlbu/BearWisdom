// =============================================================================
// ecosystem/ambient.rs — framework ambient-declaration path markers
//
// Build-tool / framework generated files and runtime declaration files whose
// exports the framework's compiler treats as ambient (available in user code
// without an explicit `import`): Nuxt/`unplugin-*` auto-import declarations,
// SvelteKit `$app`/`$env` ambient types, Next.js env types, and the Vue 3
// runtime declarations the SFC compiler injects (`Transition`, `defineProps`,
// …).
//
// These are *disk-location markers* (like `Ecosystem::pruned_dir_names`), not
// symbol lists — they say "files matching this shape are ambient providers",
// and they self-gate by existence (`node_modules/vue/dist/*.d.ts` only exists
// when Vue is installed). Kept here, in the ecosystem layer that owns external
// on-disk discovery, rather than hardcoded inside the generic resolver's
// classification path.
// =============================================================================

/// A path is an ambient provider when it contains `contains` AND ends with
/// `ends_with`. An empty `contains` matches any path (suffix-only rule).
pub struct AmbientPathMarker {
    pub contains: &'static str,
    pub ends_with: &'static str,
}

impl AmbientPathMarker {
    /// `path` is expected pre-lowercased with `\` normalised to `/`.
    pub fn matches(&self, normalized_lower_path: &str) -> bool {
        normalized_lower_path.contains(self.contains)
            && normalized_lower_path.ends_with(self.ends_with)
    }
}

/// Framework-generated and runtime ambient declaration markers. Matched
/// against a candidate file path to decide whether its symbols are ambient.
pub const FRAMEWORK_AMBIENT_MARKERS: &[AmbientPathMarker] = &[
    // Build-tool auto-import declarations (nuxt prepare, unplugin-*).
    AmbientPathMarker { contains: "/.nuxt/", ends_with: "imports.d.ts" },
    AmbientPathMarker { contains: "/.nuxt/", ends_with: "components.d.ts" },
    AmbientPathMarker { contains: "/.svelte-kit/", ends_with: "ambient.d.ts" },
    AmbientPathMarker { contains: "/.next/", ends_with: "next-env.d.ts" },
    // No-leading-slash forms for project-root-relative DB paths.
    AmbientPathMarker { contains: "", ends_with: ".nuxt/imports.d.ts" },
    AmbientPathMarker { contains: "", ends_with: ".nuxt/components.d.ts" },
    AmbientPathMarker { contains: "", ends_with: ".svelte-kit/ambient.d.ts" },
    AmbientPathMarker { contains: "", ends_with: ".next/next-env.d.ts" },
    // Vue 3 runtime declarations whose exports the SFC compiler injects.
    AmbientPathMarker { contains: "node_modules/vue/dist/", ends_with: ".d.ts" },
    AmbientPathMarker { contains: "node_modules/@vue/runtime-core/dist/", ends_with: ".d.ts" },
    AmbientPathMarker { contains: "node_modules/@vue/runtime-dom/dist/", ends_with: ".d.ts" },
    AmbientPathMarker { contains: "node_modules/@vue/reactivity/dist/", ends_with: ".d.ts" },
];

/// True when `normalized_lower_path` (pre-lowercased, `/`-normalised) matches
/// any framework ambient marker.
pub fn is_framework_ambient_path(normalized_lower_path: &str) -> bool {
    FRAMEWORK_AMBIENT_MARKERS
        .iter()
        .any(|m| m.matches(normalized_lower_path))
}

#[cfg(test)]
#[path = "ambient_tests.rs"]
mod tests;
