import init, { WasmSimulationState } from '../pkg/sandart_wasm.js';

let state = null;
let canvas = null;
let lastTime = 0;
let isDraggingCamera = false;
let isDraggingMarble = false;
let isSandFall = false;
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
let frameDurations = [];

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
    syncMaterialTheme(true);
    syncColorTheme();
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

const STANDARD_TARGETS = [
    { fps: 240, ms: 1000 / 240 },
    { fps: 165, ms: 1000 / 165 },
    { fps: 144, ms: 1000 / 144 },
    { fps: 120, ms: 1000 / 120 },
    { fps: 90,  ms: 1000 / 90 },
    { fps: 75,  ms: 1000 / 75 },
    { fps: 60,  ms: 1000 / 60 },
    { fps: 30,  ms: 1000 / 30 }
];

function detectTargetFrameTime(durations) {
    if (durations.length < 10) {
        return 1000 / 60; // Default to 60 FPS during startup
    }
    const counts = {};
    for (const d of durations) {
        if (d < 3.0 || d > 100.0) continue;
        // Find the nearest standard target
        let nearest = STANDARD_TARGETS[6]; // Default to 60
        let minDist = Infinity;
        for (const t of STANDARD_TARGETS) {
            const dist = Math.abs(d - t.ms);
            if (dist < minDist) {
                minDist = dist;
                nearest = t;
            }
        }
        counts[nearest.ms] = (counts[nearest.ms] || 0) + 1;
    }
    // Find the mode (highest frequency)
    let modeMs = 1000 / 60;
    let maxCount = 0;
    for (const ms in counts) {
        if (counts[ms] > maxCount) {
            maxCount = counts[ms];
            modeMs = parseFloat(ms);
        }
    }
    return modeMs;
}

function tick(now) {
    const frameTimeMs = now - lastTime;
    const rawDt = Math.min(frameTimeMs / 1000, 0.1); // Clamp dt to prevent massive steps
    lastTime = now;

    // Track rolling history of frame durations (last 120 frames) to detect physical Vsync
    frameDurations.push(frameTimeMs);
    if (frameDurations.length > 120) {
        frameDurations.shift();
    }
    // Compute detected vsync using a robust mode-based approach
    const detectedVsyncMs = detectTargetFrameTime(frameDurations);

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
        state.step(smoothDt, cursorX, cursorY, isDraggingMarble, frameTimeMs, detectedVsyncMs);
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
        const targetFps = 1000.0 / detectedVsyncMs;
        document.getElementById('stat-fps').innerText = `FPS: ${frameCount} (EMA: ${emaMs.toFixed(1)} ms, Target: ${targetFps.toFixed(0)} FPS, Budget N: ${budgetN})`;
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
        if (e.shiftKey && !isSandFall) {
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

function hexToRgbBytes(hex) {
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    return [r, g, b];
}

function hueToRgbBytes(h) {
    const r = Math.abs(h * 6.0 - 3.0) - 1.0;
    const g = 2.0 - Math.abs(h * 6.0 - 2.0);
    const b = 2.0 - Math.abs(h * 6.0 - 4.0);
    return [
        Math.round(Math.max(0.0, Math.min(1.0, r)) * 255.0),
        Math.round(Math.max(0.0, Math.min(1.0, g)) * 255.0),
        Math.round(Math.max(0.0, Math.min(1.0, b)) * 255.0),
    ];
}

function generateColormap(pattern, color1Hex, color2Hex) {
    const size = 512;
    const data = new Uint8Array(size * size * 4);
    const c1 = hexToRgbBytes(color1Hex);
    const c2 = hexToRgbBytes(color2Hex);

    if (pattern === 'solid') {
        for (let i = 0; i < size * size; i++) {
            data[i * 4] = c1[0];
            data[i * 4 + 1] = c1[1];
            data[i * 4 + 2] = c1[2];
            data[i * 4 + 3] = 255;
        }
    } else if (pattern === 'gradient') {
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const t = x / (size - 1);
                const idx = (y * size + x) * 4;
                data[idx] = Math.round(c1[0] * (1.0 - t) + c2[0] * t);
                data[idx + 1] = Math.round(c1[1] * (1.0 - t) + c2[1] * t);
                data[idx + 2] = Math.round(c1[2] * (1.0 - t) + c2[2] * t);
                data[idx + 3] = 255;
            }
        }
    } else if (pattern === 'stripes') {
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const t = Math.floor((x + y) / 32) % 2 === 0;
                const idx = (y * size + x) * 4;
                const color = t ? c1 : c2;
                data[idx] = color[0];
                data[idx + 1] = color[1];
                data[idx + 2] = color[2];
                data[idx + 3] = 255;
            }
        }
    } else if (pattern === 'concentric') {
        const cx = size / 2;
        const cy = size / 2;
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const dx = x - cx;
                const dy = y - cy;
                const dist = Math.sqrt(dx * dx + dy * dy);
                const t = Math.floor(dist / 32) % 2 === 0;
                const idx = (y * size + x) * 4;
                const color = t ? c1 : c2;
                data[idx] = color[0];
                data[idx + 1] = color[1];
                data[idx + 2] = color[2];
                data[idx + 3] = 255;
            }
        }
    } else if (pattern === 'checkerboard') {
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const t = (Math.floor(x / 32) % 2 === 0) !== (Math.floor(y / 32) % 2 === 0);
                const idx = (y * size + x) * 4;
                const color = t ? c1 : c2;
                data[idx] = color[0];
                data[idx + 1] = color[1];
                data[idx + 2] = color[2];
                data[idx + 3] = 255;
            }
        }
    } else if (pattern === 'rainbow_linear') {
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const hue = x / (size - 1);
                const rgb = hueToRgbBytes(hue);
                const idx = (y * size + x) * 4;
                data[idx] = rgb[0];
                data[idx + 1] = rgb[1];
                data[idx + 2] = rgb[2];
                data[idx + 3] = 255;
            }
        }
    } else if (pattern === 'rainbow_radial') {
        const cx = size / 2;
        const cy = size / 2;
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const dx = x - cx;
                const dy = y - cy;
                const angle = Math.atan2(dy, dx);
                const hue = (angle + Math.PI) / (2.0 * Math.PI);
                const rgb = hueToRgbBytes(hue);
                const idx = (y * size + x) * 4;
                data[idx] = rgb[0];
                data[idx + 1] = rgb[1];
                data[idx + 2] = rgb[2];
                data[idx + 3] = 255;
            }
        }
    }
    return data;
}

function syncColorTheme() {
    const patternSelect = document.getElementById('color-pattern');
    const presetSelect = document.getElementById('color-preset');
    const colorInput1 = document.getElementById('color-sand-1');
    const colorInput2 = document.getElementById('color-sand-2');
    const colorInput2Wrapper = document.getElementById('color-sand-2-wrapper');
    const customDiv = document.getElementById('custom-color-inputs');

    if (!patternSelect || !presetSelect || !colorInput1 || !colorInput2) return;

    const pattern = patternSelect.value;
    const preset = presetSelect.value;

    // 1. Handle presets override
    if (preset === 'rainbow') {
        patternSelect.value = 'rainbow_linear';
        customDiv.style.display = 'none';
    } else if (preset !== 'custom') {
        customDiv.style.display = 'block';
        if (preset === 'black_white') {
            colorInput1.value = '#ffffff';
            colorInput2.value = '#050505';
        } else if (preset === 'desert') {
            colorInput1.value = '#ebd9bb';
            colorInput2.value = '#8b5a2b';
        } else if (preset === 'ocean') {
            colorInput1.value = '#008080';
            colorInput2.value = '#e0f7fa';
        } else if (preset === 'forest') {
            colorInput1.value = '#228b22';
            colorInput2.value = '#ffd700';
        } else if (preset === 'vaporwave') {
            colorInput1.value = '#ff007f';
            colorInput2.value = '#00f0ff';
        } else if (preset === 'sunset') {
            colorInput1.value = '#ff4500';
            colorInput2.value = '#ffd700';
        }
    } else {
        customDiv.style.display = 'block';
    }

    // 2. Handle pattern inputs visibility
    const updatedPattern = patternSelect.value;
    if (updatedPattern === 'solid') {
        colorInput2Wrapper.style.display = 'none';
    } else if (updatedPattern === 'rainbow_linear' || updatedPattern === 'rainbow_radial') {
        customDiv.style.display = 'none';
    } else {
        customDiv.style.display = 'block';
        colorInput2Wrapper.style.display = 'flex';
    }

    // 3. Update WASM state color mode
    if (state) {
        const isSolid = updatedPattern === 'solid';
        state.set_color_mode(isSolid ? 0 : 1);
        
        const c1 = hexToRgb(colorInput1.value);
        state.set_sand_color(c1[0], c1[1], c1[2]);

        const colormapData = generateColormap(updatedPattern, colorInput1.value, colorInput2.value);
        state.update_colormap(colormapData);
    }
}

// 4. Multi-Material Property Configuration
const MATERIAL_PRESETS = {
    0: [0.00, 0.08, 0.25, 0.45], // DrySand
    1: [0.20, 0.10, 0.15, 0.35], // KineticSand
    2: [0.45, 0.14, 0.08, 0.40], // WetSand
    3: [0.00, 0.11, 0.22, 0.80], // CoarseSand
    4: [0.70, 0.04, 0.15, 0.08], // ButterCream
    5: [0.05, 0.15, 0.20, 0.20], // Snow
    6: [0.00, 0.05, 0.30, 0.05], // FinePowder
    7: [0.55, 0.04, 0.12, 0.15], // Oobleck
    8: [0.00, 0.20, 0.20, 0.10], // MoonDust
    10: [1.00, 0.00, 0.00, 0.00], // Water
    11: [0.95, 0.00, 0.00, 0.00], // Milk
    13: [0.85, 0.00, 0.00, 0.00], // VegetableOil
    14: [0.90, 0.00, 0.00, 0.00], // CalmWater
    15: [0.75, 0.00, 0.00, 0.08]  // Yogurt
};

const MATERIAL_LABELS = {
    0: "Dry Sand",
    1: "Kinetic Sand",
    2: "Wet Sand",
    3: "Coarse Sand",
    4: "Butter-Cream",
    5: "Snow",
    6: "Fine Powder",
    7: "Oobleck",
    8: "Moon Dust",
    10: "Water",
    11: "Milk",
    13: "Vegetable Oil",
    14: "Calm Water",
    15: "Yogurt"
};

function generateMaterialProps(pattern, mat1Id, mat2Id) {
    const size = 512;
    const data = new Float32Array(size * size * 4);
    const m1 = MATERIAL_PRESETS[mat1Id] || MATERIAL_PRESETS[0];
    const m2 = MATERIAL_PRESETS[mat2Id] || MATERIAL_PRESETS[0];

    if (pattern === 'solid') {
        for (let i = 0; i < size * size; i++) {
            data[i * 4 + 0] = m1[0];
            data[i * 4 + 1] = m1[1];
            data[i * 4 + 2] = m1[2];
            data[i * 4 + 3] = m1[3];
        }
    } else if (pattern === 'gradient') {
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const t = x / (size - 1);
                const idx = (y * size + x) * 4;
                data[idx + 0] = m1[0] * (1.0 - t) + m2[0] * t;
                data[idx + 1] = m1[1] * (1.0 - t) + m2[1] * t;
                data[idx + 2] = m1[2] * (1.0 - t) + m2[2] * t;
                data[idx + 3] = m1[3] * (1.0 - t) + m2[3] * t;
            }
        }
    } else if (pattern === 'stripes') {
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const t = Math.floor((x + y) / 32) % 2 === 0;
                const idx = (y * size + x) * 4;
                const m = t ? m1 : m2;
                data[idx + 0] = m[0];
                data[idx + 1] = m[1];
                data[idx + 2] = m[2];
                data[idx + 3] = m[3];
            }
        }
    } else if (pattern === 'concentric') {
        const cx = size / 2;
        const cy = size / 2;
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const dx = x - cx;
                const dy = y - cy;
                const dist = Math.sqrt(dx * dx + dy * dy);
                const t = Math.floor(dist / 32) % 2 === 0;
                const idx = (y * size + x) * 4;
                const m = t ? m1 : m2;
                data[idx + 0] = m[0];
                data[idx + 1] = m[1];
                data[idx + 2] = m[2];
                data[idx + 3] = m[3];
            }
        }
    } else if (pattern === 'checkerboard') {
        for (let y = 0; y < size; y++) {
            for (let x = 0; x < size; x++) {
                const t = (Math.floor(x / 32) % 2 === 0) !== (Math.floor(y / 32) % 2 === 0);
                const idx = (y * size + x) * 4;
                const m = t ? m1 : m2;
                data[idx + 0] = m[0];
                data[idx + 1] = m[1];
                data[idx + 2] = m[2];
                data[idx + 3] = m[3];
            }
        }
    }
    return data;
}

function renderMaterialPreview(m1Id, m2Id, isBlend) {
    const previewDiv = document.getElementById('material-properties-preview');
    if (!previewDiv) return;

    const m1 = MATERIAL_PRESETS[m1Id];
    const m2 = MATERIAL_PRESETS[m2Id];
    if (!m1) return;

    let html = '';
    if (!isBlend) {
        html += `<div class="material-preview-title">Profile: ${MATERIAL_LABELS[m1Id]}</div>`;
        html += renderPropBar("💧 Wetness", m1[0]);
        html += renderPropBar("📐 Repose", m1[1], 0.25);
        html += renderPropBar("🏎️ Flow Rate", m1[2]);
        html += renderPropBar("🌾 Grain Size", m1[3]);
    } else {
        html += `<div class="material-preview-title">Blend Profiles</div>`;
        html += `<div style="color: var(--text-muted); margin-bottom: 6px; font-size: 10px;">${MATERIAL_LABELS[m1Id]} ➔ ${MATERIAL_LABELS[m2Id]}</div>`;
        html += renderPropBar("💧 Wetness", m1[0], 1.0, m2[0]);
        html += renderPropBar("📐 Repose", m1[1], 0.25, m2[1]);
        html += renderPropBar("🏎️ Flow Rate", m1[2], 1.0, m2[2]);
        html += renderPropBar("🌾 Grain Size", m1[3], 1.0, m2[3]);
    }
    previewDiv.innerHTML = html;
}

function renderPropBar(label, val, maxVal = 1.0, blendVal = null) {
    const percent = Math.round((val / maxVal) * 100);
    const blendPercent = blendVal !== null ? Math.round((blendVal / maxVal) * 100) : null;
    
    let barStyle = `width: ${percent}%;`;
    if (blendPercent !== null) {
        barStyle = `width: 100%; background: linear-gradient(90deg, var(--accent-color) ${percent}%, #00f0ff ${blendPercent}%);`;
    }

    const displayVal = blendVal !== null ? `${val.toFixed(2)}➔${blendVal.toFixed(2)}` : val.toFixed(2);

    return `
        <div class="material-prop-bar-container">
            <div class="material-prop-label">${label}</div>
            <div class="material-prop-bar-outer">
                <div class="material-prop-bar-inner" style="${barStyle}"></div>
            </div>
            <div class="material-prop-val">${displayVal}</div>
        </div>
    `;
}

function syncMaterialTheme(resetBuffer = false) {
    const patternSelect = document.getElementById('material-pattern');
    const mat1Select = document.getElementById('material-1');
    const mat2Select = document.getElementById('material-2');
    const mat2Group = document.getElementById('material-2-group');

    if (!patternSelect || !mat1Select || !mat2Select) return;

    const pattern = patternSelect.value;
    const mat1Id = parseInt(mat1Select.value);
    const mat2Id = parseInt(mat2Select.value);

    const isBlend = pattern !== 'solid';
    if (isBlend) {
        mat2Group.style.display = 'block';
    } else {
        mat2Group.style.display = 'none';
    }

    renderMaterialPreview(mat1Id, mat2Id, isBlend);

    if (state) {
        state.set_material_mode(mat1Id);
        if (resetBuffer) {
            const propData = generateMaterialProps(pattern, mat1Id, mat2Id);
            state.set_cell_props(propData);
        }
    }
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
    state.set_sandbox_shape(parseInt(document.getElementById('shape-select').value));
    state.set_led_mode(parseInt(document.getElementById('led-mode').value));

    // Colors
    const ledColor = hexToRgb(document.getElementById('color-led').value);
    state.set_led_color(ledColor[0], ledColor[1], ledColor[2]);

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

    const selects = ['led-mode'];
    selects.forEach(id => {
        const el = document.getElementById(id);
        if (el) el.addEventListener('change', syncSettings);
    });

    const matSelects = ['material-pattern', 'material-1', 'material-2'];
    matSelects.forEach(id => {
        const el = document.getElementById(id);
        if (el) {
            el.addEventListener('change', () => {
                syncSettings();
                syncMaterialTheme(true);
            });
        }
    });

    // Preset Swatches Click Listeners
    document.querySelectorAll('.swatch').forEach(sw => {
        sw.addEventListener('click', () => {
            document.querySelectorAll('.swatch').forEach(s => s.classList.remove('active'));
            sw.classList.add('active');
            const preset = sw.dataset.preset;
            const presetSelect = document.getElementById('color-preset');
            if (presetSelect) {
                presetSelect.value = preset;
                syncColorTheme();
            }
        });
    });

    document.getElementById('shape-select').addEventListener('change', () => {
        syncSettings();
        loadActivePattern();
    });

    document.getElementById('pattern-select').addEventListener('change', () => {
        syncSettings();
        loadActivePattern();
    });

    document.getElementById('color-led').addEventListener('change', syncSettings);

    const colorThemeControls = ['color-pattern', 'color-preset', 'color-sand-1', 'color-sand-2'];
    colorThemeControls.forEach(id => {
        const el = document.getElementById(id);
        if (el) {
            el.addEventListener('change', syncColorTheme);
            el.addEventListener('input', syncColorTheme);
        }
    });

    document.getElementById('check-shadows').addEventListener('change', syncSettings);

    // Operations buttons
    document.getElementById('btn-reset').addEventListener('click', () => {
        if (state) {
            state.reset();
            syncMaterialTheme(true);
        }
    });

    document.getElementById('btn-ripples').addEventListener('click', () => {
        if (state) state.draw_ripples();
    });

    document.getElementById('btn-load-pattern').addEventListener('click', () => {
        syncSettings();
        loadActivePattern();
    });

    // Tab switching event listeners
    const tabSandbox = document.getElementById('tab-sandbox');
    const tabSandFall = document.getElementById('tab-sandfall');
    const sandboxMarbles = document.getElementById('sandbox-only-marbles');
    const sandboxPatterns = document.getElementById('sandbox-only-patterns');
    const sandfallControls = document.getElementById('sandfall-controls');

    function switchMode(mode) {
        if (!state) return;
        if (mode === 'sandbox') {
            isSandFall = false;
            tabSandbox.classList.add('active');
            tabSandFall.classList.remove('active');
            sandboxMarbles.style.display = 'block';
            sandboxPatterns.style.display = 'block';
            sandfallControls.style.display = 'none';

            // Tell WASM to switch to Sandbox mode
            state.set_simulator_mode(0);
            
            // Restore gravity to zero (standard sandbox behavior)
            state.set_gravity(0.0, 0.0);
            
            // Sync settings and patterns
            syncSettings();
            loadActivePattern();
            syncMaterialTheme(true);
        } else if (mode === 'sandfall') {
            isSandFall = true;
            tabSandbox.classList.remove('active');
            tabSandFall.classList.add('active');
            sandboxMarbles.style.display = 'none';
            sandboxPatterns.style.display = 'none';
            sandfallControls.style.display = 'block';

            // Tell WASM to switch to Sand-fall mode
            state.set_simulator_mode(1);
            
            // Sync gravity and neck width
            syncSandFallSettings();
            syncMaterialTheme(true);
        }
    }

    tabSandbox.addEventListener('click', () => switchMode('sandbox'));
    tabSandFall.addEventListener('click', () => switchMode('sandfall'));

    // Flip Hourglass Button
    document.getElementById('btn-flip').addEventListener('click', () => {
        if (state && isSandFall) {
            state.flip_hourglass();
        }
    });

    // Gravity Strength Slider
    const gravitySlider = document.getElementById('gravity-slider');
    const gravityVal = document.getElementById('gravity-val');
    const neckSlider = document.getElementById('neck-slider');
    const curvatureSlider = document.getElementById('curvature-slider');
    
    function syncSandFallSettings() {
        if (!state) return;
        const val = parseFloat(gravitySlider.value);
        gravityVal.innerText = val.toFixed(3);
        // Force downward gravity (0.0, strength)
        state.set_gravity(0.0, val);

        const neckVal = parseFloat(neckSlider.value);
        document.getElementById('neck-val').innerText = neckVal.toFixed(3);
        state.set_neck_width(neckVal);

        const curveVal = parseFloat(curvatureSlider.value);
        document.getElementById('curvature-val').innerText = curveVal.toFixed(1);
        state.set_hourglass_curve(curveVal);
    }
    gravitySlider.addEventListener('input', syncSandFallSettings);

    // Neck Width Slider
    neckSlider.addEventListener('input', () => {
        syncSandFallSettings();
        // Since changing neck width changes the boundary, reset the simulation to re-initialize the hourglass bed
        state.reset();
        syncMaterialTheme(true);
    });

    // Curvature Slider
    curvatureSlider.addEventListener('input', () => {
        syncSandFallSettings();
        // Since changing curvature changes the boundary, reset the simulation to re-initialize the hourglass bed
        state.reset();
        syncMaterialTheme(true);
    });

    // Toggle Sidebar
    const sidebar = document.getElementById('settings-sidebar');
    const toggleBtn = document.getElementById('toggle-sidebar');
    toggleBtn.addEventListener('click', () => {
        sidebar.classList.toggle('collapsed');

        // Animate canvas resizing during the 300ms sidebar transition to expand drawing area in real-time
        const startTime = performance.now();
        const duration = 300; // matches CSS transition duration

        function animateResize(now) {
            handleResize();
            if (now - startTime < duration) {
                requestAnimationFrame(animateResize);
            } else {
                handleResize(); // Final call to guarantee exact match
            }
        }
        requestAnimationFrame(animateResize);
    });
}

// Start execution
start().catch(err => {
    console.error("Initialization error:", err);
    alert("WebGL2/WebGPU initialize failed or browser does not support: " + err);
});
