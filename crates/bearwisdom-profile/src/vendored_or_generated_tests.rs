use super::*;

#[test]
fn node_modules_segment_is_vendor() {
    assert_eq!(
        classify("node_modules/some-pkg/dist/index.d.ts"),
        Some(VendorOrGeneratedKind::Vendor)
    );
}

#[test]
fn vendor_segment_is_vendor() {
    assert_eq!(
        classify("vendor/guzzlehttp/psr7/src/Uri.php"),
        Some(VendorOrGeneratedKind::Vendor)
    );
}

#[test]
fn third_party_segment_is_vendor() {
    assert_eq!(
        classify("third_party/zlib/inflate.c"),
        Some(VendorOrGeneratedKind::Vendor)
    );
    assert_eq!(
        classify("third-party/zlib/inflate.c"),
        Some(VendorOrGeneratedKind::Vendor)
    );
}

#[test]
fn dist_segment_is_generated() {
    assert_eq!(
        classify("dist/bundle.js"),
        Some(VendorOrGeneratedKind::Generated)
    );
}

#[test]
fn build_segment_is_generated() {
    assert_eq!(
        classify("build/Release/binding.node"),
        Some(VendorOrGeneratedKind::Generated)
    );
}

#[test]
fn framework_output_segments_are_generated() {
    for path in [
        ".next/static/chunk.js",
        ".nuxt/dist/client.js",
        ".svelte-kit/output/client/app.js",
        ".output/server/index.mjs",
        "generated/api_client.py",
        "__generated__/schema.ts",
        "obj/Debug/net8.0/App.dll",
        "bin/Debug/net8.0/App.exe",
        "target/debug/app",
        ".gradle/caches/foo.jar",
    ] {
        assert_eq!(
            classify(path),
            Some(VendorOrGeneratedKind::Generated),
            "{path} should classify as Generated"
        );
    }
}

#[test]
fn generated_filename_suffixes() {
    for path in [
        "Models.g.cs",
        "Models.designer.cs",
        "Api.generated.cs",
        "Api.generated.ts",
        "types.gen.go",
        "message.pb.go",
        "service_pb2.py",
        "service_pb2_grpc.py",
        "bundle.min.js",
        "app.bundle.js",
    ] {
        assert_eq!(
            classify(path),
            Some(VendorOrGeneratedKind::Generated),
            "{path} should classify as Generated"
        );
    }
}

#[test]
fn gradle_buildsrc_is_not_a_near_miss() {
    // `buildSrc/` is real Gradle project code, not the `build/` output dir.
    assert_eq!(classify("buildSrc/src/main/kotlin/Deps.kt"), None);
}

#[test]
fn distributed_rs_is_not_a_near_miss() {
    // A file literally named `distributed.rs` must not match the `dist`
    // segment pattern — segment matching is exact, not prefix.
    assert_eq!(classify("src/distributed.rs"), None);
}

#[test]
fn builder_dir_is_not_a_near_miss() {
    // A `builder/` directory must not match the `build` segment pattern.
    assert_eq!(classify("src/builder/mod.rs"), None);
}

#[test]
fn ordinary_source_file_does_not_match() {
    assert_eq!(classify("src/app.ts"), None);
    assert_eq!(classify("lib/models.py"), None);
}

#[test]
fn windows_separators_are_normalized() {
    assert_eq!(
        classify("dist\\bundle.min.js"),
        Some(VendorOrGeneratedKind::Generated)
    );
}
