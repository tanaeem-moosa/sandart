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

#### Installing Required System Dependencies

On Steam Deck, basic development libraries might be missing. If compilation fails due to missing graphics libraries (like `vulkan`, `wayland`, or `x11` headers):
- **Option A (Nix-User-Chroot / Nix)**: Use Nix package manager in user space to manage development environments without modifying the root filesystem.
- **Option B (Distrobox / Toolbx)**: Steam Deck comes pre-installed with `distrobox`. You can create an Arch, Fedora, or Ubuntu container where you have full root/sudo access to install headers, and build the binary there:
  ```bash
  distrobox create --name rust-dev --image archlinux:latest
  distrobox enter rust-dev
  # Inside distrobox, you can run sudo pacman -Syu base-devel ...
  ```

---

## Development Roadmap

- [ ] **Phase 1: Project Scaffolding**: Setup Cargo project, add windowing and rendering skeleton.
- [ ] **Phase 2: Basic Sand Rendering**: Implement a heightmap canvas with a mouse-controlled ball that clears pixels.
- [ ] **Phase 3: Physics & Settling**: Implement the sand sliding / angle-of-repose algorithm.
- [ ] **Phase 4: Mathematical Pattern Generators**: Create a system for path generation over time.
- [ ] **Phase 5: UI & Settings Panel**: Build the egui overlay for parameter tuning.
- [ ] **Phase 6: Advanced Shading & LEDs**: Write custom shaders for grain textures and RGB rim lights.
- [ ] **Phase 7: File Import**: Parse and render `.thr` (Theta-Rho) files.
