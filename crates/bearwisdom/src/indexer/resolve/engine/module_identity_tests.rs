use super::*;

#[test]
fn project_granularity_shares_one_container() {
    // Every symbol gets the empty discriminator — same key regardless of file,
    // so resolution behaves exactly as it did before module identity.
    assert_eq!(module_discriminator(ModuleGranularity::Project, "a/b.ts"), "");
    assert_eq!(module_discriminator(ModuleGranularity::Project, "c/d.ts"), "");
}

#[test]
fn file_granularity_uses_the_declaring_file() {
    // Two files exporting the same name get distinct discriminators, so their
    // symbols resolve as distinct.
    let a = module_discriminator(ModuleGranularity::File, "packages/react-query/src/QueryClientProvider.tsx");
    let b = module_discriminator(ModuleGranularity::File, "packages/vue-query/src/useQueryClient.ts");
    assert_eq!(a, "packages/react-query/src/QueryClientProvider.tsx");
    assert_ne!(a, b);
}
