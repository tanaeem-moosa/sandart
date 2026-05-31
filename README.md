# Sands of Time: Kinetic Sand Art Simulator

A beautiful, high-performance simulation of a kinetic sand art table (like the Sisyphus table) written in Rust. The application simulates a physical steel ball rolling through a sand bed, carving intricate mathematical and path-based designs, illuminated by a dynamic RGB LED ring.

![Sand Art Table Design Concept](https://images.unsplash.com/photo-1618005182384-a83a8bd57fbe?auto=format&fit=crop&w=800&q=80) *Note: Placeholder illustration for visual concept.*

## Project Goals

1. **Realistic Sand Physics & Displacement**:
   - **Heightmap Simulation**: Simulate the sand bed using a dynamic 2D heightmap.
   - **Displacement**: As the marble rolls, it pushes sand outward, creating realistic grooves and side ridges.
   - **Settle/Slide Effect**: Simulate gravity pulling sand back down if slopes exceed the natural angle of repose.
   
2. **Stunning Visuals & Lighting**:
   - **Height-Based Shading**: Real-time phong shading or normal mapping of the sand surface to render realistic shadows, specular highlights on sand grains, and ambient occlusion.
   - **Dynamic RGB LED Ring**: Customize a multi-point LED lighting system around the circular rim (simulating the color gradients seen in physical kinetic tables).
   - **Customizable Styles**: Choose sand color, grain texture, LED brightness, and shadow softness.

3. **Intricate Pattern Generation**:
   - **Mathematical Patterns**: Built-in generators for Spirographs, Lissajous curves, Rose curves, Trochoids, and Fourier-series-based art.
   - **Theta-Rho (`.thr`) File Support**: Support loading and playing standard `.thr` coordinate files widely used by physical kinetic sand tables.
   - **Interactive Drawing**: Drag the marble manually with the mouse/touchscreen to draw custom paths in real-time.

4. **Modern, Responsive User Interface**:
   - Built-in control panel to adjust ball speed, size, pattern parameters, color profiles, LED animations, and physics constants.
   - Cross-platform support (runs natively on Linux, Steam Deck, Windows, macOS, and potentially WebAssembly).

---

## Architecture & Tech Stack

- **Language**: Rust 🦀
- **Graphics & Rendering**: 
  - `wgpu` or `pixels` for hardware-accelerated GPU rendering.
  - Custom fragment shader for sand surface heightmap reconstruction, lighting (normal map generation), and LED ring ambient illumination.
- **User Interface**: `egui` (via `eframe`) for a clean, lightweight, immediate-mode GUI.
- **Physics**: Lightweight cellular automata or grid-based heightmap filters written in Rust (parallelized with `rayon` or run on the GPU via compute shaders).

---

## Getting Started

### Prerequisites

To compile and run this project, you need the Rust toolchain. Since the Steam Deck runs an immutable OS, we install and run everything inside **user space** without using `sudo`.

#### Installing Rust in User Space (Steam Deck / Immutable Linux)

You can easily install Rust locally using `rustup`, which installs entirely within your home directory (`~/.cargo` and `~/.rustup`).

1. Open a terminal and run the official `rustup` installer:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   ```
2. Configure your current shell session to include Cargo's binary directory on your `PATH`:
   ```bash
   source "$HOME/.cargo/env"
   ```
3. (Optional) To make this persistent, ensure your shell profile (e.g., `~/.bashrc` or `~/.zshrc`) loads the environment automatically. The installer usually appends this, but if not, you can manually add:
   ```bash
   export PATH="$HOME/.cargo/bin:$PATH"
   ```

#### Installing Required System Dependencies via Distrobox

Since the Steam Deck's root filesystem is read-only and standard system package modifications are wiped during SteamOS updates, we compile inside a **Distrobox** container named `sandart-dev` using an Arch Linux image. This container runs in user-space and has full access to developer libraries.

1. **Create the container**:
   ```bash
   distrobox create --name sandart-dev --image archlinux:latest
   ```
2. **Enter the container**:
   ```bash
   distrobox enter sandart-dev
   ```
3. **Install build tools and graphics development libraries (inside container)**:
   ```bash
   sudo pacman -Syu --noconfirm base-devel pkgconf mesa libx11 libxrandr libxi libxcursor wayland libxkbcommon
   ```
4. **Install or load Rust (inside container)**:
   ```bash
   # If Rust was not already installed:
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   
   # Load cargo onto the path:
   source "$HOME/.cargo/env"
   ```

To compile or run the application from the host command line, you can execute:
```bash
distrobox enter sandart-dev -- /home/deck/.cargo/bin/cargo run --release
```

Once compiled, you can copy the binary from `target/release/sandart` directly to your `$HOME/.local/bin/` directory to run it natively on the Steam Deck host outside of Distrobox.

---

## Development Roadmap (Reviewed & Refined)

The project will be built in incremental, testable blocks:

- [ ] **Block 1: Basic Scaffolding & Windowing**: Set up the `egui` layout, menus, panel widgets, and a placeholder canvas utilizing `egui::Painter` to verify layout resizing and responsiveness.
- [ ] **Block 2: WGPU Render Pipeline Hook**: Integrate `egui_wgpu` with custom paint callbacks. Compile and run a basic vertex/fragment shader rendering a flat, colored circle to prove GPU synchronization.
- [ ] **Block 3: Heightmap Texture & CPU Buffer Transfer**: Create a $512 \times 512$ float grid on the CPU. Map it to a dynamic WGPU texture, upload it each frame, and display it as a grayscale canvas.
- [ ] **Block 4: Coordinate Mapping & Path Drawing**: Map GUI coordinate space to heightmap coordinate space. Track mouse click-and-drag and draw simple trails in the sand.
- [ ] **Block 5A: Marble Path Interpolation & Volume-conserving Displacement**: Prevent "dotted line" trails at high speeds by interpolating paths. Displace sand volume by pushing it into surrounding side-ridges instead of erasing it.
- [ ] **Block 5B: CPU Heightmap Settling (Cellular Automata)**: Implement gravity settling using local slopes (angle of repose) inside an active bounding box, capped at a stable transfer rate ($\alpha < 0.25$) to prevent rendering flicker.
- [ ] **Block 6: Spiral Generator & Path Follower**: Implement polar coordinate movement algorithms to auto-play Archimedean spirals of variable sizing and density.
- [ ] **Block 7: GPU 3D Normal Shading & Raymarched Shadows**: Update the fragment shader to calculate surface normals dynamically on the GPU (sampling neighbor pixels) and render realistic 3D Phong shadows and specular sand highlights.

