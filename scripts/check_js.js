// Automated static and runtime scope checker for sandart-wasm/web/demo.js
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const jsPath = path.resolve(__dirname, '../sandart-wasm/web/demo.js');
console.log(`Checking ${jsPath}...`);

let code = fs.readFileSync(jsPath, 'utf8');
// Replace ES module imports with no-op in VM sandbox
code = code.replace(/import\s+.*?from\s+['"].*?['"];?/g, '// import removed');

// Mock browser DOM and WASM environment for initialization validation
const mockElements = new Map();
function getMockElement(id) {
    if (!mockElements.has(id)) {
        mockElements.set(id, {
            id,
            value: '0',
            innerText: '',
            textContent: '',
            style: {},
            dataset: {},
            classList: {
                add() {},
                remove() {},
                toggle() {},
                contains() { return false; }
            },
            addEventListener(event, handler) {},
            querySelectorAll() { return []; },
            appendChild() {},
            cloneNode() { return getMockElement(id + '_clone'); },
            getBoundingClientRect() { return { width: 800, height: 600 }; }
        });
    }
    return mockElements.get(id);
}

const mockDocument = {
    getElementById(id) {
        return getMockElement(id);
    },
    querySelectorAll(selector) {
        return [];
    },
    createElement(tag) {
        return getMockElement('created_' + tag);
    }
};

const mockWasmState = {
    list_materials() {
        return [
            ['dry_sand', 'Dry Sand', 0.0, 0.5, 0.8, 1.0],
            ['water', 'Water', 1.0, 0.0, 1.0, 0.0]
        ];
    },
    build_git_sha() { return 'test-sha'; },
    build_timestamp_epoch() { return 1700000000; },
    get_grid_size() { return 128; },
    get_multistage_chambers() { return 8; },
    neck_half_width_cells() { return 2.0; },
    set_sandbox_shape() {},
    set_simulator_mode() {},
    set_gravity() {},
    set_neck_width() {},
    set_hourglass_curve() {},
    set_multistage_chambers() {},
    set_quantile_mode() {},
    set_pattern_mode() {},
    load_preset_pattern() {},
    set_active_color_theme() {},
    set_primary_palette() {},
    set_secondary_palette() {},
    set_cell_props() {},
    reset() {},
    flip_hourglass() {},
    update() {},
    render() {}
};

const sandbox = {
    console,
    window: {
        addEventListener() {},
        devicePixelRatio: 1.0,
        innerWidth: 800,
        innerHeight: 600
    },
    document: mockDocument,
    performance: { now: () => 0 },
    requestAnimationFrame() {},
    WasmSimulationState: mockWasmState,
    init: async () => {},
    alert(msg) { console.warn("Alert called:", msg); },
    Float32Array,
    Uint8Array,
    Uint32Array,
    Map,
    Set,
    Math,
    parseInt,
    parseFloat,
    String,
    Date
};

vm.createContext(sandbox);

try {
    vm.runInContext(code, sandbox);
    console.log("✓ Syntax and top-level execution PASSED");

    // Test calling functions to ensure all internal references exist
    if (typeof sandbox.switchMode === 'function') {
        sandbox.switchMode('sandfall');
        sandbox.switchMode('sandbox');
        console.log("✓ switchMode() execution PASSED");
    }

    if (typeof sandbox.populateMaterialSelects === 'function') {
        sandbox.populateMaterialSelects();
        console.log("✓ populateMaterialSelects() execution PASSED");
    }

    if (typeof sandbox.updateChambersRowVisibility === 'function') {
        sandbox.updateChambersRowVisibility();
        console.log("✓ updateChambersRowVisibility() execution PASSED");
    }

    if (typeof sandbox.updateVesselReadouts === 'function') {
        sandbox.updateVesselReadouts();
        console.log("✓ updateVesselReadouts() execution PASSED");
    }

    // ---- index.html structural checks -------------------------------------------------
    //
    // Added 2026-08-31 after a cleanup left one unmatched </div> in the Debug group. That closed
    // #app-container early, so #viewport-container (and the canvas inside it) became a direct
    // child of <body> instead of a flex child of #app-container -- the page rendered nothing and
    // the sidebar contents spilled over the render area. Everything else here passed, including
    // the whole Rust test suite, because nothing in this file looked at the HTML.
    const htmlPath = path.join(__dirname, '..', 'sandart-wasm', 'web', 'index.html');
    const html = fs.readFileSync(htmlPath, 'utf8');

    // 1. Tag nesting. Only <div> is tracked: it is the tag the layout depends on and the one an
    //    edit is most likely to unbalance.
    const tagRe = /<(\/?)div\b[^>]*>/g;
    let depth = 0, m;
    while ((m = tagRe.exec(html)) !== null) {
        depth += m[1] ? -1 : 1;
        if (depth < 0) {
            const line = html.slice(0, m.index).split('\n').length;
            throw new Error(`index.html: unmatched </div> at line ${line} (closes past the root)`);
        }
    }
    if (depth !== 0) {
        throw new Error(`index.html: ${depth} <div> left unclosed at EOF`);
    }
    console.log("✓ index.html <div> nesting PASSED");

    // 2. The canvas must stay inside #app-container. This is the invariant the bug above broke,
    //    asserted directly rather than inferred from tag counts.
    const appIdx = html.indexOf('id="app-container"');
    const viewIdx = html.indexOf('id="viewport-container"');
    if (appIdx < 0 || viewIdx < 0) {
        throw new Error('index.html: #app-container or #viewport-container is missing');
    }
    // Count from the START of each element's own <div tag, so both opening tags are whole.
    const appOpen = html.lastIndexOf('<div', appIdx);
    const viewOpen = html.lastIndexOf('<div', viewIdx);
    let d = 0;
    const between = html.slice(appOpen, viewOpen);
    const t2 = /<(\/?)div\b[^>]*>/g;
    while ((m = t2.exec(between)) !== null) d += m[1] ? -1 : 1;
    // d is the nesting depth of #viewport-container relative to <body>: >= 1 means it is still
    // inside #app-container. 0 or less means a stray </div> has closed the container early.
    if (d < 1) {
        throw new Error(
            `index.html: #viewport-container escaped #app-container (relative depth ${d}) -- ` +
            'a stray </div> has closed the container early; the canvas will render nothing'
        );
    }
    console.log("✓ index.html canvas nesting PASSED");

    // 3. Every getElementById(...) in demo.js must resolve. This catches the other half of the
    //    same failure mode: deleting a control's markup but leaving the JS that reads it, or
    //    deleting live markup while cleaning up something unrelated.
    const jsIds = new Set();
    const idRe = /getElementById\(\s*['"]([^'"]+)['"]\s*\)/g;
    while ((m = idRe.exec(code)) !== null) jsIds.add(m[1]);
    const htmlIds = new Set();
    const hIdRe = /\bid="([^"]+)"/g;
    while ((m = hIdRe.exec(html)) !== null) htmlIds.add(m[1]);
    const missing = [...jsIds].filter((id) => !htmlIds.has(id)).sort();
    if (missing.length) {
        throw new Error(
            'demo.js reads element ids that do not exist in index.html: ' + missing.join(', ')
        );
    }
    console.log(`✓ demo.js -> index.html id resolution PASSED (${jsIds.size} ids)`);

    console.log("All JS pre-commit checks PASSED successfully!");
    process.exit(0);
} catch (err) {
    console.error("❌ JS Pre-commit validation FAILED:", err);
    process.exit(1);
}
