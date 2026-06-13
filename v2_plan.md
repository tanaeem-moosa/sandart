# Sand Art Simulator: Version 2 Plan

This document captures the design, module division, and implementation roadmap for v2 of the Sand Art Simulator, focused on modular refactoring and Web/WASM support.

---

## Goal 1: Decoupled Simulation & Physics Crate (`sandart-sim`)

**Feasibility: High.**

Extract the physics grid, cellular automata updates, heightmap settling, and drawing algorithms out of the main desktop app loop. We will expose a generic `Simulation` trait so that future physics engines (such as a 3D gravity-driven hourglass simulation) can be swapped in seamlessly.

**Key Design Decisions:**
- Create `sandart-sim` as a library crate.
- Define a unified `Simulation` trait for all physics grids.
- Keep the physics engine entirely independent of the graphics API and the GUI framework.
- Retain all material presets (Butter-Cream, Dry Sand, Snow, Kinetic Sand, etc.) inside the simulation physics loop.

---

## Goal 2: Decoupled WGPU Renderer Crate (`sandart-render`)

**Feasibility: High.**

Extract all graphics pipeline creation, vertex/index buffer management, and shader operations into a standalone rendering library. 

**Key Design Decisions:**
- Create `sandart-render` as a library crate that manages `shader.wgsl` and WGPU rendering resources.
- Eliminate all `egui` and `eframe` dependencies from the renderer. The renderer will operate on raw WGPU types (device, queue, texture formats, and render passes).
- Decouple from any specific simulation crate by passing raw primitives (like float height slices and simple coordinates) rather than depending on the concrete simulation structures.
- Expose APIs to allow rendering directly to any target surface, supporting both native desktop viewports and HTML5 browser canvases.

---

## Goal 3: Decoupled Pattern Crate (`sandart-pattern`)

**Feasibility: High.**

Isolate the path calculations and G-code/THR file parsers to keep the simulation and rendering libraries lightweight and clean.

**Key Design Decisions:**
- Create `sandart-pattern` as a standalone mathematical crate.
- Remove all local disk file system access (`std::fs`) from the parser functions. The parsing functions will accept string slices (`&str`), delegating actual file fetching or reading to the runner (UI app or web host), ensuring full compatibility with the browser sandbox.

---

## Goal 4: Web/WASM Bindings Crate (`sandart-wasm`)

**Feasibility: Medium.**

Provide WASM bindings to compile the simulator core, rendering code, and math pattern logic to target `wasm32-unknown-unknown` via `wasm-bindgen`.

**Key Design Decisions:**
- Create `sandart-wasm` containing the WASM-bindgen interface.
- Expose simulation updates and rendering execution directly to JavaScript.
- Provide a simple web harness (`index.html` and `demo.js`) in the workspace to load, tick, and render the simulator onto an HTML5 `<canvas>` using WebGL2 or WebGPU.

---

## Goal 5: Egui Desktop GUI Crate (`sandart`)

**Feasibility: High.**

Update the existing desktop application to act as the main GUI runner, pulling in the modularized libraries and composing them inside the eframe/egui panel structure.

**Key Design Decisions:**
- Convert the root crate to a binary crate named `sandart`.
- Implement `egui_wgpu::CallbackTrait` in the desktop app to bridge the custom standalone WGPU heightmap renderer and the egui painter canvas.
- Keep all UI controls, camera mouse controls, and native file dialogues in this crate.

---

## Refactoring Roadmap & Agent Implementation Guide

Follow these steps incrementally, running tests and launching the native desktop application at each commit verification boundary to guarantee correctness.

### Step 1: Workspace Scaffolding [Complete]
*   **Action:**
    1. Create subdirectories: `sandart-sim/src`, `sandart-render/src`, `sandart-pattern/src`, `sandart-wasm/src`, and `sandart/src`.
    2. Set up their individual `Cargo.toml` manifests with appropriate library configurations.
    3. Modify root `Cargo.toml` to serve as the workspace manifest:
       ```toml
       [workspace]
       members = [
           "sandart-sim",
           "sandart-render",
           "sandart-pattern",
           "sandart-wasm",
           "sandart"
       ]
       resolver = "2"
       ```
*   **Verification Checkpoint:** Run `cargo check --workspace` to ensure scaffolding resolves correctly.

---

### Step 2: Migrate Patterns Crate (`sandart-pattern`) [Complete]
*   **Action:**
    1. Move `src/pattern.rs` to `sandart-pattern/src/lib.rs`.
    2. Decouple it entirely from UI configurations (`crate::config`) or simulation references.
    3. **Critical WASM Compliance:** Ensure file parsing functions (`parse_gcode`, `parse_thr`) take `&str` instead of loading files using `std::fs` directly. The calling runner (desktop UI or web host) is responsible for loading the file contents and passing the raw text.
    4. Move relevant pattern-generation tests to `sandart-pattern/src/lib.rs`.
    5. Add `sandart-pattern = { path = "../sandart-pattern" }` as a dependency to the root package and redirect references inside root code.
*   **Verification Checkpoint:** Run `cargo test -p sandart-pattern` and run the desktop app to ensure mathematical patterns generate and load exactly as before. Commit.

---

### Step 3: Migrate Simulation Crate (`sandart-sim`) [Complete]
*   **Action:**
    1. Move `src/sim.rs` and `src/sim/` folder to `sandart-sim/src/`.
    2. Migrate `MaterialMode` and `SandboxShape` enums into `sandart-sim`.
    3. Define the `HeightmapSimulation` trait:
       ```rust
       pub trait HeightmapSimulation {
           fn update(&mut self, dt: f32, cursor_targets: &[Option<glam::Vec2>]);
           fn reset(&mut self);
           fn heightmap(&self) -> &[f32];
           fn dimensions(&self) -> (usize, usize);
           fn marbles(&self) -> &[MarbleState; 5];
           fn active_bounds(&self) -> ActiveBounds;
       }
       ```
    4. **Performance Checkpoint:** Refactor the hot update loops in `physics.rs` and `sim.rs` to avoid heap allocations. The `marbles` method must return a reference to a fixed-size stack array (`&[MarbleState; 5]`) to prevent frame-by-frame `Vec` allocations.
    5. Expose `DrawingSimulation` implementing the trait. Move sim tests to `sandart-sim`.
    6. Add `sandart-sim` as a dependency to the root package and adapt imports.
*   **Verification Checkpoint:** Run `cargo test -p sandart-sim` and run the desktop app to ensure all sand physics presets are functioning properly. Commit.

---

### Step 4: Migrate Standalone Renderer Crate (`sandart-render`) [Complete]
*   **Action:**
    1. Move `src/renderer.rs` and `src/shader.wgsl` to `sandart-render/src/`.
    2. Remove all `egui` and `eframe` references, converting it into a pure WGPU renderer.
    3. Rename resources to `HeightmapRenderer`.
    4. **Performance Checkpoint:** Implement sub-rectangle GPU texture updates:
       ```rust
       pub fn update_heightmap_partial(&mut self, queue: &wgpu::Queue, data: &[f32], bounds: ActiveBounds)
       ```
       Inside this function, calculate the offset into the full `data` float array and use `queue.write_texture` with custom `origin`, `offset`, and strided `bytes_per_row` to copy **only the settled/active sub-rectangle**, skipping full 4MB uploads.
    5. Expose a raw `draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, camera: &CameraUniforms, light: &LightingUniforms)` method.
    6. Integrate `sandart-render = { path = "../sandart-render" }` into the root dependencies.
*   **Verification Checkpoint:** Run `cargo test -p sandart-render` (confirming pipeline creation and headless capture passes), compile and test the app. Commit.

---

### Step 5: Adapt Desktop App UI Crate (`sandart`) [Complete]
*   **Action:**
    1. Move the remaining desktop files `src/main.rs`, `src/app.rs`, and `src/config.rs` into the `sandart/src/` subdirectory.
    2. Delete the old root `src/` directory.
    3. In `sandart/src/app.rs`, implement `egui_wgpu::CallbackTrait` wrapping the `HeightmapRenderer` and uploading camera/lighting uniforms.
    4. **Performance Checkpoint (Zero-Mutex Direct GPU Updates):**
       In `SandArtApp::update()`, get the renderer callback resource directly via `frame.wgpu_render_state().renderer.write()` and call `update_heightmap_partial` on it directly using `sim.heightmap()` and `sim.active_bounds()`.
       *Eliminate `shared_heightmap: Arc<Mutex<Vec<f32>>>` and its CPU-to-CPU copy.*
*   **Verification Checkpoint:** Run `cargo test --workspace` and compile/run `cargo run --bin sandart` to verify the entire desktop application is working cleanly. Commit.

---

### Step 6: Create WASM Bindings Crate (`sandart-wasm`) [Complete]
*   **Action:**
    1. Write `wasm-bindgen` bindings in `sandart-wasm/src/lib.rs` wrapping `sandart-sim`, `sandart-pattern`, and `sandart-render`.
    2. **WASM Compatibility Checkpoint (Non-Blocking async):** Ensure WGPU device creation in WASM is asynchronous (`async fn init_wgpu`), returning a Promise to JavaScript, as blocking on threads (`pollster::block_on`) panics in web environments.
    3. **Performance Checkpoint (Zero-Copy):** Expose raw views of the heightmap using `js_sys::Float32Array::view(&sim.heightmap())` to allow JS to read values directly from WASM memory.
    4. Provide a simple `index.html` and `demo.js` under `sandart-wasm/web/` showing how to instantiate the engine and render to a canvas.
*   **Verification Checkpoint:** Build for the web:
    ```bash
    wasm-pack build sandart-wasm --target web
    ```
    Ensure the compiled JS/WASM bundle initializes and runs successfully inside a browser canvas. Commit.

---

### Step 7: Update Documentation
*   **Action:**
    Update `v1_plan.md` and `handover.md` in the root folder to map all old module/file references to their new paths in the workspace crates.
*   **Verification Checkpoint:** Compile, test, and perform a final check.
