import init, { WasmSimulationState } from './pkg/sandart_wasm.js';

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
let smoothDt = null;
let totalStepTime = 0;
let totalRenderTime = 0;
let renderTimeCount = 0;

async function start() {
    // Initialize WASM module
    await init();

    canvas = document.getElementById('sand-canvas');
    
    // Adjust size for High DPI screens
    let rect = canvas.getBoundingClientRect();
    // Cap DPR to 1.5 to prevent massive rendering performance hit on 4K/high-res displays
    const dpr = Math.min(window.devicePixelRatio || 1, 1.5);
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
    loadActivePattern();

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
    // Cap DPR to 1.5 to prevent massive rendering performance hit on 4K/high-res displays
    const dpr = Math.min(window.devicePixelRatio || 1, 1.5);
    const w = Math.round(rect.width * dpr);
    const h = Math.round(rect.height * dpr);
    canvas.width = w;
    canvas.height = h;
    if (state) {
        state.resize(w, h);
    }
}

function tick(now) {
    const rawDt = Math.min((now - lastTime) / 1000, 0.1); // Clamp dt to prevent massive steps
    lastTime = now;

    // Smooth delta-time using Exponential Moving Average (EMA) to eliminate browser timer resolution jitter
    if (smoothDt === null) {
        smoothDt = rawDt > 0.0 ? rawDt : 0.01666;
    } else if (rawDt > 0.0) {
        if (rawDt > 0.1) {
            // Reset smoothDt on huge frame gaps (e.g. after tab focus loss)
            smoothDt = rawDt;
        } else {
            // Apply standard EMA filter (90% history, 10% new frame)
            smoothDt = smoothDt * 0.9 + rawDt * 0.1;
        }
    }

    // Step physics & render
    if (state) {
        const startStep = performance.now();
        const frameTimeMs = now - lastTime;
        state.step(smoothDt, cursorX, cursorY, isDraggingMarble, frameTimeMs);
        const stepTime = performance.now() - startStep;

        const startRender = performance.now();
        state.render();
        const renderTime = performance.now() - startRender;

        totalStepTime += stepTime;
        totalRenderTime += renderTime;
        renderTimeCount++;
    }

    // Calculate FPS and average frame time, update UI once per second to prevent DOM thrashing
    frameCount++;
    if (now - fpsTime >= 1000) {
        const avgStepTime = renderTimeCount > 0 ? (totalStepTime / renderTimeCount) : 0;
        const avgRenderTime = renderTimeCount > 0 ? (totalRenderTime / renderTimeCount) : 0;
        const avgTotalTime = avgStepTime + avgRenderTime;
        
        // Update sidebar stats
        const budgetN = state.get_budget_n();
        const emaMs = state.get_ema_frame_ms();
        document.getElementById('stat-fps').innerText = `FPS: ${frameCount} (EMA: ${emaMs.toFixed(1)} ms, Budget N: ${budgetN})`;
        document.getElementById('stat-render-time').innerText = `Frame time: ${avgTotalTime.toFixed(1)} ms (CPU: ${avgStepTime.toFixed(1)} ms, GPU: ${avgRenderTime.toFixed(1)} ms)`;
        
        const blockCounts = state.get_active_block_counts();
        const inactive = blockCounts[0];
        const slow = blockCounts[1];
        const medium = blockCounts[2];
        const fast = blockCounts[3];
        document.getElementById('stat-blocks').innerText = `Blocks: Must(F): ${fast}, Budget(M): ${medium}, Stale(S): ${slow}, Inactive(I): ${inactive}`;
        
        // Update floating HUD stats
        const hudFps = document.getElementById('hud-fps');
        const hudTime = document.getElementById('hud-time');
        if (hudFps && hudTime) {
            hudFps.innerText = `${frameCount} FPS`;
            hudTime.innerText = `${avgTotalTime.toFixed(1)} ms (CPU: ${avgStepTime.toFixed(1)}ms, GPU: ${avgRenderTime.toFixed(1)}ms)`;
        }

        frameCount = 0;
        totalStepTime = 0;
        totalRenderTime = 0;
        renderTimeCount = 0;
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

    // Update dynamic parameter panels visibility & slider constraints (does not reset/reload pattern)
    const patternType = document.getElementById('pattern-select').value;
    updateParamPanels(patternType);
}

function updateParamPanels(type) {
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
}

function loadActivePattern() {
    if (!state) return;
    const type = document.getElementById('pattern-select').value;
    if (type === 'manual') {
        state.set_pattern_mode('Manual');
        return;
    }
    if (type === 'clock') {
        state.set_pattern_mode('Clock');
    } else {
        state.set_pattern_mode('Pattern');
    }
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

    const selects = ['material-select', 'led-mode'];
    selects.forEach(id => {
        document.getElementById(id).addEventListener('change', syncSettings);
    });

    document.getElementById('shape-select').addEventListener('change', () => {
        syncSettings();
        loadActivePattern();
    });

    document.getElementById('pattern-select').addEventListener('change', () => {
        syncSettings();
        loadActivePattern();
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

    document.getElementById('btn-load-pattern').addEventListener('click', () => {
        syncSettings();
        loadActivePattern();
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
