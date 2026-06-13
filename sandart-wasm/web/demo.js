import init, { WasmSimulationState } from '../pkg/sandart_wasm.js';

let state = null;
let canvas = null;
let lastTime = 0;
let isDraggingCamera = false;
let isDraggingMarble = false;
let mouseX = 0;
let mouseY = 0;
let cursorX = 0;
let cursorY = 0;

// Camera state parameters matching desktop defaults
let cameraAzimuth = 0.0;
let cameraElevation = 0.8;
let cameraZoom = 2.8;

// Event loop variables
let frameCount = 0;
let fpsTime = 0;

async function start() {
    // Initialize WASM module
    await init();

    canvas = document.getElementById('sand-canvas');
    
    // Adjust size for High DPI screens
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;

    // Create simulator state
    state = await WasmSimulationState.create('sand-canvas', canvas.width, canvas.height);

    // Initial config sync
    syncSettings();
    updateCamera();

    // Hook up event listeners
    window.addEventListener('resize', handleResize);
    setupCanvasInput();
    setupPanelInput();

    // Start requestAnimationFrame loop
    lastTime = performance.now();
    fpsTime = lastTime;
    requestAnimationFrame(tick);
}

function handleResize() {
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(rect.width * dpr);
    const h = Math.round(rect.height * dpr);
    canvas.width = w;
    canvas.height = h;
    if (state) {
        state.resize(w, h);
    }
}

function tick(now) {
    const dt = Math.min((now - lastTime) / 1000, 0.1); // Clamp dt to prevent massive steps
    lastTime = now;

    // Step physics & render
    if (state) {
        const startRender = performance.now();
        state.step(dt, cursorX, cursorY, isDraggingMarble);
        state.render();
        const renderTime = performance.now() - startRender;
        document.getElementById('stat-render-time').innerText = `Frame time: ${renderTime.toFixed(1)} ms`;
    }

    // Calculate FPS
    frameCount++;
    if (now - fpsTime >= 1000) {
        document.getElementById('stat-fps').innerText = `FPS: ${frameCount}`;
        frameCount = 0;
        fpsTime = now;
    }

    requestAnimationFrame(tick);
}

// Map screen space mouse coordinates to sand bed circular coordinates
function getMouseCoordinates(e) {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    
    // Scale canvas dimensions to center square
    const side = Math.min(rect.width, rect.height);
    const cx = rect.width / 2;
    const cy = rect.height / 2;
    
    // Convert to NDC coordinate (-1 to 1) relative to sand bed circle
    const ndc_x = (x - cx) / (side / 2);
    const ndc_y = -(y - cy) / (side / 2);
    return { x: ndc_x, y: ndc_y };
}

function updateCamera() {
    if (state) {
        state.set_camera(cameraAzimuth, cameraElevation, cameraZoom);
    }
}

function setupCanvasInput() {
    canvas.addEventListener('mousedown', (e) => {
        if (e.shiftKey) {
            // Drag the magnet/marble
            isDraggingMarble = true;
            const pos = getMouseCoordinates(e);
            cursorX = pos.x;
            cursorY = pos.y;
        } else {
            // Drag the camera
            isDraggingCamera = true;
            mouseX = e.clientX;
            mouseY = e.clientY;
        }
    });

    window.addEventListener('mousemove', (e) => {
        if (isDraggingMarble) {
            const pos = getMouseCoordinates(e);
            cursorX = pos.x;
            cursorY = pos.y;
        } else if (isDraggingCamera) {
            const dx = e.clientX - mouseX;
            const dy = e.clientY - mouseY;
            mouseX = e.clientX;
            mouseY = e.clientY;

            // Update camera angles
            cameraAzimuth += dx * 0.007;
            cameraElevation = Math.max(0.1, Math.min(Math.PI / 2 - 0.05, cameraElevation - dy * 0.007));
            updateCamera();
        }
    });

    window.addEventListener('mouseup', () => {
        isDraggingCamera = false;
        isDraggingMarble = false;
    });

    canvas.addEventListener('wheel', (e) => {
        cameraZoom = Math.max(1.2, Math.min(5.0, cameraZoom + e.deltaY * 0.0015));
        updateCamera();
        e.preventDefault();
    }, { passive: false });
}

function hexToRgb(hex) {
    const r = parseInt(hex.slice(1, 3), 16) / 255;
    const g = parseInt(hex.slice(3, 5), 16) / 255;
    const b = parseInt(hex.slice(5, 7), 16) / 255;
    return [r, g, b];
}

function syncSettings() {
    if (!state) return;

    // Sliders
    const speed = parseFloat(document.getElementById('speed-slider').value);
    state.set_speed(speed);
    document.getElementById('speed-val').innerText = `${speed.toFixed(2)} R/s`;

    const size = parseFloat(document.getElementById('size-slider').value);
    state.set_marble_size(size);
    document.getElementById('size-val').innerText = `${size.toFixed(3)} R`;

    const count = parseInt(document.getElementById('marble-count').value);
    state.set_marble_count(count);
    document.getElementById('count-val').innerText = `${count}`;

    // Selects
    state.set_material_mode(parseInt(document.getElementById('material-select').value));
    state.set_sandbox_shape(parseInt(document.getElementById('shape-select').value));
    state.set_led_mode(parseInt(document.getElementById('led-mode').value));

    // Colors
    const ledColor = hexToRgb(document.getElementById('color-led').value);
    state.set_led_color(ledColor[0], ledColor[1], ledColor[2]);

    const sandColor = hexToRgb(document.getElementById('color-sand').value);
    state.set_sand_color(sandColor[0], sandColor[1], sandColor[2]);

    // Lighting Angle & Shadows
    state.set_light_angle(parseFloat(document.getElementById('angle-slider').value));
    state.set_shadows_enabled(document.getElementById('check-shadows').checked);
}

function generatePattern(type) {
    if (!state) return;
    
    if (type === 'manual') {
        state.set_pattern_mode('Manual');
        return;
    }

    state.set_pattern_mode('Pattern');
    
    let content = '';
    const arms = parseInt(document.getElementById('marble-count').value);

    if (type === 'spiral') {
        const spacing = 0.035;
        const max_r = 0.874;
        const a = spacing / (2.0 * Math.PI);
        const total_theta = max_r / a;
        const turns = total_theta / (2.0 * Math.PI);
        const steps = Math.ceil(turns * 128);
        for (let i = 0; i <= steps; i++) {
            const t = i / steps;
            const theta = t * total_theta;
            const rho = (a * theta) / 0.874;
            content += `${theta.toFixed(5)} ${rho.toFixed(5)}\n`;
        }
        state.load_multi_pattern('thr', content, arms);
    } 
    else if (type === 'lissajous') {
        const steps = 1500;
        for (let i = 0; i <= steps; i++) {
            const t = (i / steps) * 2 * Math.PI * 10;
            const x = Math.sin(3 * t);
            const y = Math.sin(4 * t);
            content += `G1 X${(x * 10).toFixed(3)} Y${(y * 10).toFixed(3)}\n`;
        }
        state.load_multi_pattern('gcode', content, arms);
    } 
    else if (type === 'rose') {
        const steps = 1500;
        const theta_max = 2 * Math.PI * 8;
        for (let i = 0; i <= steps; i++) {
            const theta = (i / steps) * theta_max;
            const rho = Math.cos(3.5 * theta);
            content += `${theta.toFixed(5)} ${rho.toFixed(5)}\n`;
        }
        state.load_multi_pattern('thr', content, arms);
    } 
    else if (type === 'spirograph') {
        const steps = 2000;
        const theta_max = 2 * Math.PI * 16;
        const ri = 0.62;
        const d = 0.38;
        for (let i = 0; i <= steps; i++) {
            const theta = (i / steps) * theta_max;
            const x = (1.0 - ri) * Math.cos(theta) + d * Math.cos(((1.0 - ri) / ri) * theta);
            const y = (1.0 - ri) * Math.sin(theta) - d * Math.sin(((1.0 - ri) / ri) * theta);
            content += `G1 X${(x * 10).toFixed(3)} Y${(y * 10).toFixed(3)}\n`;
        }
        state.load_multi_pattern('gcode', content, arms);
    } 
    else if (type === 'gosper') {
        // Simple Gosper L-system generator in JS
        const rules = {
            'A': 'A-B--B+A++AA+B-',
            'B': '+A-BB--B-A++A+B'
        };
        let current = 'A';
        const depth = 3;
        for (let d = 0; d < depth; d++) {
            let next = '';
            for (let char of current) {
                next += rules[char] || char;
            }
            current = next;
        }

        let x = 0;
        let y = 0;
        let angle = 0;
        const step = 0.2;
        content += `G1 X${x.toFixed(3)} Y${y.toFixed(3)}\n`;
        for (let char of current) {
            if (char === 'A' || char === 'B') {
                x += Math.cos(angle) * step;
                y += Math.sin(angle) * step;
                content += `G1 X${x.toFixed(3)} Y${y.toFixed(3)}\n`;
            } else if (char === '+') {
                angle += Math.PI / 3; // +60 deg
            } else if (char === '-') {
                angle -= Math.PI / 3; // -60 deg
            }
        }
        state.load_multi_pattern('gcode', content, arms);
    }
    else if (type === 'hilbert') {
        // Recursive Hilbert path generation
        const path = [];
        const hilbert = (x, y, xi, xj, yi, yj, n) => {
            if (n === 0) {
                path.push({ x: x + (xi + yi) / 2, y: y + (xj + yj) / 2 });
            } else {
                hilbert(x, y, yi / 2, yj / 2, xi / 2, xj / 2, n - 1);
                hilbert(x + xi / 2, y + xj / 2, xi / 2, xj / 2, yi / 2, yj / 2, n - 1);
                hilbert(x + xi / 2 + yi / 2, y + xj / 2 + yj / 2, xi / 2, xj / 2, yi / 2, yj / 2, n - 1);
                hilbert(x + xi / 2 + yi, y + xj / 2 + yj, -yi / 2, -yj / 2, -xi / 2, -xj / 2, n - 1);
            }
        };
        hilbert(-5, -5, 10, 0, 0, 10, 4);
        for (let p of path) {
            content += `G1 X${p.x.toFixed(3)} Y${p.y.toFixed(3)}\n`;
        }
        state.load_multi_pattern('gcode', content, arms);
    }
}

function setupPanelInput() {
    // Input sync listeners
    const sliders = ['speed-slider', 'size-slider', 'marble-count', 'angle-slider'];
    sliders.forEach(id => {
        document.getElementById(id).addEventListener('input', syncSettings);
    });

    const selects = ['material-select', 'shape-select', 'led-mode'];
    selects.forEach(id => {
        document.getElementById(id).addEventListener('change', syncSettings);
    });

    const colors = ['color-led', 'color-sand'];
    colors.forEach(id => {
        document.getElementById(id).addEventListener('change', syncSettings);
    });

    document.getElementById('check-shadows').addEventListener('change', syncSettings);

    // Operations buttons
    document.getElementById('btn-reset').addEventListener('click', () => {
        if (state) state.reset();
    });

    document.getElementById('btn-ripples').addEventListener('click', () => {
        if (state) state.draw_ripples();
    });

    // Pattern select listener
    document.getElementById('pattern-select').addEventListener('change', (e) => {
        generatePattern(e.target.value);
    });

    // Toggle Sidebar
    const sidebar = document.getElementById('settings-sidebar');
    const toggleBtn = document.getElementById('toggle-sidebar');
    toggleBtn.addEventListener('click', () => {
        sidebar.classList.toggle('collapsed');
    });
}

// Start execution
start().catch(err => {
    console.error("Initialization error:", err);
    alert("WebGL2/WebGPU initialize failed or browser does not support: " + err);
});
