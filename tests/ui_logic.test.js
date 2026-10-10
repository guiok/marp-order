// Testes das funções puras da interface (o bloco entre <pure> e </pure> de src/ui.html).
// Rodar com: node --test tests/ui_logic.test.jsui_logic.test.js
'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const html = fs.readFileSync(path.join(__dirname, '..', 'src', 'ui.html'), 'utf8');
const block = html.slice(html.indexOf('// <pure>'), html.indexOf('// </pure>'));
const names = [...block.matchAll(/^(?:function|const) (\w+)/gm)].map((m) => m[1]);
const ui = new Function(`${block}\nreturn { ${names.join(', ')} };`)();

const slides = (...texts) => texts.map((text) => ({ text, title: ui.slideTitle(text) }));
const kinds = (rows) => rows.map((row) => row.kind).join(' ');

test('slideTitle prefers a heading and falls back to the first text line', () => {
  assert.equal(ui.slideTitle('<!-- _class: lead -->\n\n## **Hello** `world`\n\ntext'), 'Hello world');
  assert.equal(ui.slideTitle('![bg](a.png)\n\nloose text\n'), 'loose text');
  assert.equal(ui.slideTitle(''), '');
});

test('lineDiff marks removed and added lines', () => {
  const diff = ui.lineDiff('# A\n\n- one\n- two', '# A\n\n- one\n- 2\n- three');
  assert.deepEqual(diff.map((l) => `${l.kind}:${l.text}`),
    ['same:# A', 'same:', 'same:- one', 'del:- two', 'add:- 2', 'add:- three']);
  assert.deepEqual(ui.lineDiff('same\r\n', 'same'), [{ kind: 'same', text: 'same' }]);
});

test('similarity is the share of common lines', () => {
  assert.equal(ui.similarity('a\nb', 'a\nb'), 1);
  assert.equal(ui.similarity('a\nb', 'c\nd'), 0);
  assert.equal(ui.similarity('a\nb\nc\nd', 'a\nb\nc\nx'), 0.75);
});

test('alignSlides finds identical, changed, removed and added slides', () => {
  const left = slides('# A', '# B\n\nold', '# Gone\n\nnothing alike', '# D');
  const right = slides('# A', '# B\n\nnew', '# D', '# New\n\nfresh');
  const rows = ui.alignSlides(left, right);
  assert.equal(kinds(rows), 'same changed left same right');
  assert.deepEqual(rows[1], { kind: 'changed', l: 1, r: 1 });
  assert.deepEqual(rows[4], { kind: 'right', r: 3, at: 4 });
});

test('alignSlides pairs retitled slides by similar content and keeps moved ones apart', () => {
  const body = '\n\n- first point\n- second point\n- third point';
  const rows = ui.alignSlides(slides('# Plan' + body, '# Z'), slides('# Roadmap' + body, '# Z'));
  assert.equal(kinds(rows), 'changed same');
  // uma lâmina que só mudou de lugar aparece como removida de um ponto e nova em outro
  assert.equal(kinds(ui.alignSlides(slides('# A', '# B', '# C'), slides('# C', '# A', '# B'))), 'right same same left');
  assert.equal(kinds(ui.alignSlides([], slides('# A'))), 'right');
});

test('splitNotes separates speaker notes from directives and code', () => {
  const text = '<!-- _class: lead -->\n\n# Title\n\n<!-- remember to smile -->\n\n```html\n<!-- code -->\n```\n\n<!--\nsecond note\n-->';
  const { body, notes } = ui.splitNotes(text);
  assert.equal(body, '<!-- _class: lead -->\n\n# Title\n\n```html\n<!-- code -->\n```');
  assert.equal(notes, 'remember to smile\n\nsecond note');
  assert.deepEqual(ui.splitNotes('# No notes'), { body: '# No notes', notes: '' });
  assert.deepEqual(ui.splitNotes('<!--\npaginate: true\n_footer: x\n-->\ntext').notes, '');
});

test('joinNotes puts the notes back as a comment', () => {
  assert.equal(ui.joinNotes('# T\n', 'say hi'), '# T\n\n<!--\nsay hi\n-->');
  assert.equal(ui.joinNotes('# T', '  '), '# T');
  const again = ui.splitNotes(ui.joinNotes('# T', 'a --> b'));
  assert.deepEqual(again, { body: '# T', notes: 'a -- > b' });
});

test('highlight keeps the text unchanged', () => {
  const text = '# Title <b>\n\n- item `code` & [link](u)\n1. one\n---\n```js\nlet a = "<";\n```\n<!-- note\nmore -->\nplain **bold**';
  const html = ui.highlight(text);
  const plain = html.replace(/<[^>]+>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
  assert.equal(plain, text);
  assert.match(html, /<span class="hl-hd"># Title &lt;b&gt;<\/span>/);
  assert.match(html, /<span class="hl-code">let a = "&lt;";<\/span>/);
  assert.match(html, /<span class="hl-cm">more --&gt;<\/span>\nplain/);
  assert.match(html, /<span class="hl-mk">-<\/span> item <span class="hl-code">`code`<\/span>/);
});

test('makeRemap carries the undo history over a save that renumbers slides', () => {
  const oldSlides = [{ text: '# A' }, { text: '# B' }, { text: '# C' }];
  // gravado: C, A editada, B  ->  o servidor renumera para 0, 1, 2
  const saved = [{ k: 1, id: 2 }, { k: 2, id: 0, text: '# A2' }, { k: 3, id: 1 }];
  const newSlides = [{ text: '# C' }, { text: '# A2' }, { text: '# B' }];
  const remap = ui.makeRemap(oldSlides, newSlides, saved, 0);
  assert.deepEqual(saved.map(remap), [{ k: 1, id: 0 }, { k: 2, id: 1 }, { k: 3, id: 2 }]);
  // estado anterior do histórico: A ainda sem a edição
  assert.deepEqual(remap({ k: 2, id: 0 }), { k: 2, id: 1, text: '# A' });
  // lâmina colada que não foi gravada continua sendo uma lâmina nova
  assert.deepEqual(remap({ k: 9, id: 0, text: '# X', fresh: true }), { k: 9, id: 0, text: '# X', fresh: true });
  // uma edição que dividiu a lâmina: sem posição confiável, vale o texto
  const split = ui.makeRemap(oldSlides, [{ text: '# A' }, { text: '# B' }, { text: '# B2' }, { text: '# C' }], saved, 0);
  assert.deepEqual(split({ k: 3, id: 2 }), { k: 3, id: 3 });
  assert.deepEqual(split({ k: 4, id: 0, text: '# gone' }), { k: 4, id: 0, text: '# gone', fresh: true });
});

test('matchesQuery needs every word, in any case', () => {
  assert.ok(ui.matchesQuery('Quarterly Roadmap\n- Q1: pilot', 'roadmap PILOT'));
  assert.ok(!ui.matchesQuery('Quarterly Roadmap', 'roadmap budget'));
  assert.ok(ui.matchesQuery('anything', '  '));
});
