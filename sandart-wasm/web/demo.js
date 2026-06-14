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
    let rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    let w = Math.round(rect.width * dpr);
    let h = Math.round(rect.height * dpr);

    if (w === 0 || h === 0) {
        const fallbackWidth = canvas.clientWidth || (window.innerWidth - 340);
        const fallbackHeight = canvas.clientHeight || window.innerHeight;
        w = Math.round(fallbackWidth * dpr);
        h = Math.round(fallbackHeight * dpr);
    }

    canvas.width = w;
    canvas.height = h;
    console.log(`Canvas size initialized to ${w}x${h} (DPR: ${dpr})`);

    // Check if WebGL is forced via URL query parameters
    const urlParams = new URLSearchParams(window.location.search);
    const forceWebGL = urlParams.has('webgl') || urlParams.has('webgl2') || urlParams.get('backend') === 'webgl';
    console.log("Initializing WasmSimulationState. Force WebGL:", forceWebGL);

    // Create simulator state
    state = await WasmSimulationState.create('sand-canvas', w, h, forceWebGL);

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
            cameraAzimuth -= dx * 0.007;
            cameraElevation = Math.max(0.1, Math.min(Math.PI / 2 - 0.05, cameraElevation + dy * 0.007));
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

    // Pattern Specific Sliders
    const spiralSpacing = parseFloat(document.getElementById('spiral-spacing-slider').value);
    state.set_spiral_spacing(spiralSpacing);
    document.getElementById('spiral-spacing-val').innerText = spiralSpacing.toFixed(3);

    const lissajousA = parseFloat(document.getElementById('lissajous-a-slider').value);
    const lissajousB = parseFloat(document.getElementById('lissajous-b-slider').value);
    state.set_lissajous_params(lissajousA, lissajousB);
    document.getElementById('lissajous-a-val').innerText = lissajousA.toFixed(2);
    document.getElementById('lissajous-b-val').innerText = lissajousB.toFixed(2);

    const roseK = parseFloat(document.getElementById('rose-k-slider').value);
    state.set_rose_k(roseK);
    document.getElementById('rose-k-val').innerText = roseK.toFixed(2);

    const spiroR = parseFloat(document.getElementById('spiro-r-slider').value);
    const spiroD = parseFloat(document.getElementById('spiro-d-slider').value);
    state.set_hypotrochoid_params(spiroR, spiroD);
    document.getElementById('spiro-r-val').innerText = spiroR.toFixed(2);
    document.getElementById('spiro-d-val').innerText = spiroD.toFixed(2);

    const curveOrder = parseInt(document.getElementById('curve-order-slider').value);
    state.set_hilbert_order(curveOrder);
    document.getElementById('curve-order-val').innerText = curveOrder;

    const walkSteps = parseInt(document.getElementById('walk-steps-slider').value);
    const walkSize = parseFloat(document.getElementById('walk-size-slider').value);
    state.set_random_walk_params(walkSteps, walkSize);
    document.getElementById('walk-steps-val').innerText = walkSteps;
    document.getElementById('walk-size-val').innerText = walkSize.toFixed(3);

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

    // Dynamic pattern regeneration on setting sync
    const patternType = document.getElementById('pattern-select').value;
    generatePattern(patternType);
}

function generatePattern(type) {
    if (!state) return;
    
    // Manage dynamic param visibility
    const paramIds = {
        'spiral': 'spiral-params',
        'lissajous': 'lissajous-params',
        'rose': 'rose-params',
        'fermat': 'rose-params',
        'spirograph': 'spirograph-params',
        'gosper': 'curve-params',
        'hilbert': 'curve-params',
        'sierpinski': 'curve-params',
        'random_walk': 'random-walk-params'
    };

    // Hide all
    Object.values(paramIds).forEach(id => {
        document.getElementById(id).style.display = 'none';
    });

    // Show active
    if (paramIds[type]) {
        document.getElementById(paramIds[type]).style.display = 'block';
    }

    // Dynamic styling and constraints for curve recursion depth
    if (type === 'hilbert' || type === 'gosper' || type === 'sierpinski') {
        const orderLabel = document.querySelector('label[for="curve-order-slider"]');
        const orderSlider = document.getElementById('curve-order-slider');
        const displayVal = document.getElementById('curve-order-val');
        
        let maxVal = 6;
        let labelText = 'Recursion Order (Depth)';
        
        if (type === 'hilbert') {
            maxVal = 6;
            labelText = 'Hilbert Curve Depth';
        } else if (type === 'gosper') {
            maxVal = 5;
            labelText = 'Gosper Curve Depth';
        } else if (type === 'sierpinski') {
            maxVal = 7;
            labelText = 'Sierpinski Curve Depth';
        }
        
        if (orderLabel) orderLabel.textContent = labelText;
        if (orderSlider) {
            orderSlider.max = maxVal;
            let currentVal = parseInt(orderSlider.value);
            if (currentVal > maxVal) {
                orderSlider.value = maxVal;
                state.set_hilbert_order(maxVal);
            }
            if (displayVal) {
                displayVal.innerText = orderSlider.value;
            }
        }
    }

    if (type === 'manual') {
        state.set_pattern_mode('Manual');
        return;
    }

    state.set_pattern_mode('Pattern');
    state.load_preset_pattern(type);
}

function setupPanelInput() {
    // Input sync listeners
    const sliders = [
        'speed-slider', 
        'size-slider', 
        'marble-count', 
        'angle-slider',
        'spiral-spacing-slider',
        'lissajous-a-slider',
        'lissajous-b-slider',
        'rose-k-slider',
        'spiro-r-slider',
        'spiro-d-slider',
        'curve-order-slider',
        'walk-steps-slider',
        'walk-size-slider'
    ];
    sliders.forEach(id => {
        document.getElementById(id).addEventListener('input', syncSettings);
    });

    const selects = ['material-select', 'shape-select', 'led-mode', 'pattern-select'];
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
