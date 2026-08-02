//! Static validation of `shader.wgsl`.
//!
//! `Renderer::new` compiles the shader with `create_shader_module` at RUNTIME, which means a
//! WGSL syntax or type error fails nowhere in CI and nowhere in `cargo test` — it surfaces as a
//! blank canvas in the browser. That is the single place this project has no way to verify
//! anything automatically, so shader errors were the most expensive class of mistake available
//! and the cheapest to catch.
//!
//! naga is the same validator wgpu uses internally, pinned to wgpu 23's version, so passing here
//! means the shader the browser receives will at least compile.

#[test]
fn shader_wgsl_parses_and_validates() {
    let src = include_str!("../src/shader.wgsl");

    let module = naga::front::wgsl::parse_str(src)
        .unwrap_or_else(|e| panic!("shader.wgsl failed to parse:\n{}", e.emit_to_string(src)));

    // Parsing alone only proves the syntax is well formed. Validation is what catches the
    // mistakes that actually happen when editing this file: wrong argument types, mismatched
    // vector widths, a `select` whose arms disagree, a name that does not resolve.
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("shader.wgsl failed validation:\n{e:?}"));
}

/// Guards the test above. A validator that accepts everything would pass silently forever and
/// give false confidence in exactly the situation it exists for, so prove it rejects both of the
/// failure modes it claims to cover: a syntax error, and code that parses but is ill-typed.
#[test]
fn validator_rejects_broken_shaders() {
    assert!(
        naga::front::wgsl::parse_str("fn main( { }").is_err(),
        "parser accepted a syntax error"
    );

    // Parses cleanly; `select`'s two arms disagree in type, which only validation catches.
    let ill_typed = r#"
        @fragment
        fn fs_main() -> @location(0) vec4<f32> {
            let bad = select(vec3<f32>(0.0), 1.0, true);
            return vec4<f32>(bad, 1.0);
        }
    "#;
    let parsed = naga::front::wgsl::parse_str(ill_typed);
    let rejected = match parsed {
        Err(_) => true,
        Ok(m) => naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&m)
        .is_err(),
    };
    assert!(rejected, "validator accepted a type-mismatched select");
}
