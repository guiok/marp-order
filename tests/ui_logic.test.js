// Tests for the pure functions of the interface (the block between <pure> and </pure> in src/ui.html).
// Run with: node --test tests/ui_logic.test.js
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
  // a slide that only moved shows up as removed from one place and new in another
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
  // written: C, edited A, B  ->  the server renumbers them 0, 1, 2
  const saved = [{ k: 1, id: 2 }, { k: 2, id: 0, text: '# A2' }, { k: 3, id: 1 }];
  const newSlides = [{ text: '# C' }, { text: '# A2' }, { text: '# B' }];
  const remap = ui.makeRemap(oldSlides, newSlides, saved, 0);
  assert.deepEqual(saved.map(remap), [{ k: 1, id: 0 }, { k: 2, id: 1 }, { k: 3, id: 2 }]);
  // earlier state in the history: A still without the edit
  assert.deepEqual(remap({ k: 2, id: 0 }), { k: 2, id: 1, text: '# A' });
  // a pasted slide that was not written is still a new slide
  assert.deepEqual(remap({ k: 9, id: 0, text: '# X', fresh: true }), { k: 9, id: 0, text: '# X', fresh: true });
  // an edit that split the slide: no reliable position, so the text decides
  const split = ui.makeRemap(oldSlides, [{ text: '# A' }, { text: '# B' }, { text: '# B2' }, { text: '# C' }], saved, 0);
  assert.deepEqual(split({ k: 3, id: 2 }), { k: 3, id: 3 });
  assert.deepEqual(split({ k: 4, id: 0, text: '# gone' }), { k: 4, id: 0, text: '# gone', fresh: true });
});

test('withoutItem removes a slide and picks the one to edit next', () => {
  const list = [{ k: 1 }, { k: 2 }, { k: 3 }];
  assert.deepEqual(ui.withoutItem(list, 2), { rest: [{ k: 1 }, { k: 3 }], next: { k: 3 } });
  assert.deepEqual(ui.withoutItem(list, 3), { rest: [{ k: 1 }, { k: 2 }], next: { k: 2 } });
  assert.deepEqual(ui.withoutItem([{ k: 1 }], 1), { rest: [], next: null });
  assert.deepEqual(ui.withoutItem(list, 9), { rest: list, next: null });
});

test('matchesQuery needs every word, in any case', () => {
  assert.ok(ui.matchesQuery('Quarterly Roadmap\n- Q1: pilot', 'roadmap PILOT'));
  assert.ok(!ui.matchesQuery('Quarterly Roadmap', 'roadmap budget'));
  assert.ok(ui.matchesQuery('anything', '  '));
});

test('every interface text has a Portuguese translation', () => {
  const decode = (raw) => new Function(`return '${raw}';`)();
  const keys = new Set();
  // t('...') calls in the script and data-t / data-tt / data-tp attributes in the HTML
  for (const m of html.matchAll(/\bt\('((?:[^'\\]|\\.)*)'/g)) keys.add(decode(m[1]));
  for (const m of html.matchAll(/data-t[tp]?="([^"]*)"/g)) keys.add(m[1]);
  // keys swapped at run time, as in `el.dataset.t = cond ? 'A' : 'B'`
  for (const m of html.matchAll(/dataset\.t = ([^;]*);/g)) {
    for (const q of m[1].matchAll(/'((?:[^'\\]|\\.)*)'/g)) keys.add(decode(q[1]));
  }
  for (const key of ['{0} different', '{0} only here', '{0} only in the other']) keys.add(key);
  assert.ok(keys.size > 90, `only ${keys.size} keys found`);
  const missing = [...keys].filter((key) => !(key in ui.PT));
  assert.deepEqual(missing, []);
  const unused = Object.keys(ui.PT).filter((key) => !keys.has(key));
  assert.deepEqual(unused, []);
});

test('translate fills placeholders and falls back to English', () => {
  assert.equal(ui.translate('pt', 'was {0}', [3]), 'era 3');
  assert.equal(ui.translate('en', 'was {0}', [3]), 'was 3');
  assert.equal(ui.translate('pt', '{0} copied to the end of {1}.', ['2 lâminas', 'b.md']), '2 lâminas copiada(s) para o fim de b.md.');
  assert.equal(ui.translate('pt', 'not in the dictionary', []), 'not in the dictionary');
  assert.match(ui.translate('en', 'help', []), /^Drag to reorder/);
  for (const key of Object.keys(ui.EN)) assert.ok(ui.PT[key], key);
});

test('speechChunks turns notes into plain sentences', () => {
  const notes = '## Opening\n- Greet the **room** and `smile`. Then [explain](http://x) the plan!\n\nAsk: any questions?';
  assert.deepEqual(ui.speechChunks(notes), ['Opening', 'Greet the room and smile.', 'Then explain the plan!', 'Ask:', 'any questions?']);
  assert.deepEqual(ui.speechChunks('  \n '), []);
});

test('deckLang reads the language of the front matter', () => {
  assert.equal(ui.deckLang('---\nmarp: true\nlang: pt-BR\n---\n'), 'pt-BR');
  assert.equal(ui.deckLang("---\nlang: 'en'\n---\n"), 'en');
  assert.equal(ui.deckLang('---\nmarp: true\n---\n'), '');
});

test('sortVoices puts the voices of the wanted language first', () => {
  const voices = ['en-US', 'pt-PT', 'pt-BR', 'es-ES', 'pt_BR'].map((lang, n) => ({ lang, name: `v${n}` }));
  assert.deepEqual(ui.sortVoices(voices, 'pt-BR').map((v) => v.lang), ['pt_BR', 'pt-BR', 'pt-PT', 'en-US', 'es-ES']);
  assert.deepEqual(ui.sortVoices(voices, 'en').map((v) => v.lang)[0], 'en-US');
});
