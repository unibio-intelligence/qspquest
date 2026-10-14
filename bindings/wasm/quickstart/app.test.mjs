import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

// Exercise the page handlers with a small DOM fixture and a recorded engine call.
// Browser validation itself is represented only by required and range constraints.
async function page() {
  const nodes = new Map(), calls = [];
  class Element {
    constructor(tag = 'div') {
      this.tag = tag; this.children = []; this.listeners = {}; this.dataset = {};
      this.value = ''; this.textContent = ''; this.disabled = false; this.required = false;
      this.hidden = false; this.files = []; this.namespaceURI = 'http://www.w3.org/2000/svg';
    }
    get options() { return this.tag === 'select' ? this.children : []; }
    get selectedOptions() { return this.options.filter(option => option.value === this.value); }
    append(child) { this.children.push(child); if (this.tag === 'select' && this.children.length === 1) this.value = child.value; }
    replaceChildren() { this.children = []; this.value = ''; }
    setAttribute(name, value) { this[name] = name === 'hidden' ? true : value; }
    removeAttribute(name) { if (name === 'hidden') this.hidden = false; }
    querySelectorAll() { return []; }
    addEventListener(name, handler) { this.listeners[name] = handler; }
    reportValidity() {
      return [...nodes.values()].every(el => el.disabled ||
        ((!el.required || String(el.value).trim() !== '') &&
         (el.value === '' || el.tag !== 'input' || el.type !== 'number' ||
          (Number.isFinite(Number(el.value)) && Number(el.value) >= (el.min ?? -Infinity) && Number(el.value) <= (el.max ?? Infinity)))));
    }
  }
  const html = readFileSync(new URL('./index.html', import.meta.url), 'utf8');
  for (const [, tag, attributes, id] of html.matchAll(/<([a-z]+)\b([^>]*\bid="([^"]+)"[^>]*)>/g)) {
    const el = new Element(tag);
    for (const name of ['value', 'type', 'min', 'max']) {
      const match = attributes.match(new RegExp(`\\b${name}="([^"]*)"`));
      if (match) el[name] = match[1];
    }
    for (const name of ['required', 'hidden', 'disabled']) el[name] = new RegExp(`\\b${name}\\b`).test(attributes);
    nodes.set(id, el);
  }
  const get = id => { assert.ok(nodes.has(id), `Unknown page element ${id}`); return nodes.get(id); };
  for (const value of ['synthetic', 'custom']) { const option = new Element('option'); option.value = value; get('model-picker').append(option); }
  const request = { schema: 'qspquest.run/v0.1', budgets: { output_values: 100, events: 100, solver_callbacks: 1000 }, parameters: { cl: { value: 1, unit: 'L/h' } } };
  const library = [{ name: 'Synthetic library fixture', version: '1', scenario: 'baseline', license: 'Apache-2.0', description: 'Test fixture',
    source: JSON.stringify({ model_id: 'fixture', parameters: [{ name: 'cl', description: 'Clearance', fixed: false }] }), request }];
  const context = vm.createContext({
    document: { getElementById: get, createElement: tag => new Element(tag), createElementNS: (_, tag) => new Element(tag) },
    window: { addEventListener() {} }, TextEncoder, structuredClone, URL, Blob,
    fetch: async url => ({ ok: true, text: async () => 'model source', json: async () => structuredClone(url.endsWith('library.json') ? library : request) }),
    createEngineClient: () => ({ cancel() {}, dispose() {}, run: async payload => {
      calls.push(payload);
      return { result: { times: [0, 1], columns: [[1, 0.5]], names: ['cp'], units: ['mg/L'], sides: [1, 1], time_unit: 'h', model_id: 'fixture', identity: {} } };
    } }),
  });
  const source = readFileSync(new URL('./app.mjs', import.meta.url), 'utf8');
  vm.runInContext(source.replace("import { createEngineClient } from '../js/index.mjs';", '') + '\nglobalThis.loaded = ready;', context);
  await context.loaded;
  const select = value => { get('model-picker').value = value; get('model-picker').onchange(); };
  const input = (id, value) => { get(id).value = value; (get(id).oninput || get(id).listeners.input)(); };
  const submit = () => get('inputs').listeners.submit({ preventDefault() {} });
  return { get, calls, select, input, submit, request };
}

test('blank library parameter cannot execute or retain downloadable results', async () => {
  const p = await page(); p.select('library-0');
  assert.equal(p.get('parameter-value').required, true);
  p.input('parameter-value', ''); await p.submit(); assert.equal(p.calls.length, 0);
  p.input('parameter-value', '2'); await p.submit(); assert.equal(p.calls.length, 1);
  assert.equal(p.get('download').disabled, false);
  p.input('parameter-value', ''); assert.equal(p.get('download').disabled, true);
  assert.equal(p.get('plot').hidden, true); await p.submit(); assert.equal(p.calls.length, 1);
});

test('inactive fields do not block another model mode', async () => {
  const p = await page(); p.select('library-0'); p.input('parameter-value', '');
  p.select('synthetic'); assert.equal(p.get('parameter-value').disabled, true);
  assert.equal(p.get('inputs').reportValidity(), true);
  p.input('dose', '-1'); assert.equal(p.get('inputs').reportValidity(), false);
  p.select('custom'); assert.equal(p.get('dose').disabled, true);
  assert.equal(p.get('inputs').reportValidity(), true); await p.submit(); assert.equal(p.calls.length, 1);
});

test('raw JSON edits and imported requests synchronize the parameter and comparison label', async () => {
  const p = await page(); p.select('library-0');
  const edited = structuredClone(p.request); edited.parameters.cl = { value: 9, unit: 'mL/h' };
  p.input('request', JSON.stringify(edited));
  assert.equal(Number(p.get('parameter-value').value), 9);
  assert.equal(p.get('parameter-label').textContent, 'Value (mL/h)');
  await p.submit(); assert.equal(p.calls[0].request.parameters.cl.value, 9);
  p.input('parameter-value', '10'); await p.submit();
  assert.match(p.get('comparison').textContent, /Clearance = 9/);
  edited.parameters.cl.value = 12;
  p.get('request-file').files = [{ size: 100, name: 'request.json', text: async () => JSON.stringify(edited) }];
  await p.get('request-file').onchange();
  assert.equal(Number(p.get('parameter-value').value), 12);
  await p.submit(); assert.equal(p.calls.at(-1).request.parameters.cl.value, 12);
});

test('incomplete JSON disables stale controls and corrected JSON restores them', async () => {
  const p = await page(); p.select('library-0'); p.input('request', '{');
  assert.equal(p.get('parameter-value').disabled, true);
  assert.equal(p.get('library-controls').hidden, true);
  await p.submit(); assert.equal(p.calls.length, 0);
  p.input('request', JSON.stringify(p.request));
  assert.equal(p.get('parameter-value').disabled, false);
  assert.equal(p.get('parameter-value').required, true);
  await p.submit(); assert.equal(p.calls.length, 1);
});
