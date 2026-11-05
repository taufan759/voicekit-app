"use strict";

const tauri = window.__TAURI__;
const invoke = tauri ? tauri.core.invoke : async () => { throw new Error("Jalankan lewat aplikasi VoiceKit (Tauri)."); };

/** Voice used when nothing has been picked yet, if the account has it. */
const PREFERRED_VOICE = "fin";

/** Audio tags understood by Eleven v3/v4. Labels are Indonesian, tags stay English. */
const TAGS = [
  ["excited", "Semangat"], ["happy", "Senang"], ["sad", "Sedih"], ["angry", "Marah"],
  ["curious", "Penasaran"], ["sarcastic", "Sarkas"], ["nervous", "Gugup"], ["calm", "Tenang"],
  ["whispers", "Berbisik"], ["shouts", "Berteriak"], ["laughs", "Tertawa"], ["chuckles", "Terkekeh"],
  ["sighs", "Menghela napas"], ["gasps", "Terkejut"], ["short pause", "Jeda pendek"], ["long pause", "Jeda panjang"],
];

const SLIDERS = [
  { key: "stability", label: "Stabilitas", min: 0, max: 1, step: 0.05, ends: ["Ekspresif", "Stabil"] },
  { key: "similarity_boost", label: "Kemiripan", min: 0, max: 1, step: 0.05, ends: ["Rendah", "Tinggi"] },
  { key: "style", label: "Gaya / dramatisasi", min: 0, max: 1, step: 0.05, ends: ["Datar", "Dramatis"] },
  { key: "speed", label: "Kecepatan", min: 0.7, max: 1.2, step: 0.05, ends: ["Lambat", "Cepat"] },
];

const DEFAULT_VOICE_SETTINGS = { stability: 0.5, similarity_boost: 0.75, style: 0, speed: 1, use_speaker_boost: true };
const CATEGORY_NAMES = { premade: "Default", cloned: "Suaraku", professional: "Profesional", generated: "Desain", famous: "Terkenal", high_quality: "HQ" };

const state = {
  hasKey: false,
  settings: null,
  history: [],
  models: [],
  voices: [],
  voiceFilter: "",
  cloneFiles: [],
};

const $ = (sel) => document.querySelector(sel);

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else if (v !== false && v != null) node.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) if (c != null) node.append(c);
  return node;
}

function fmt(n) { return Number(n).toLocaleString("id-ID"); }

/* ---------- feedback ---------- */

let toastTimer;
function toast(message, isError = false) {
  const box = $("#toast");
  box.textContent = message;
  box.className = isError ? "toast err" : "toast";
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => box.classList.add("hidden"), isError ? 7000 : 3500);
}

async function busy(text, work) {
  $("#busy-text").textContent = text;
  $("#busy").classList.remove("hidden");
  try { return await work(); } finally { $("#busy").classList.add("hidden"); }
}

/* ---------- theme & navigation ---------- */

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  try { localStorage.setItem("voicekit-theme", theme); } catch { /* storage unavailable */ }
}

function initTheme() {
  let saved = null;
  try { saved = localStorage.getItem("voicekit-theme"); } catch { /* storage unavailable */ }
  applyTheme(saved || (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"));
  $("#theme-btn").addEventListener("click", () =>
    applyTheme(document.documentElement.dataset.theme === "dark" ? "light" : "dark"));
}

function show(view) {
  for (const v of document.querySelectorAll(".view")) v.classList.toggle("hidden", v.id !== `view-${view}`);
  for (const b of document.querySelectorAll(".nav")) b.classList.toggle("active", b.dataset.view === view);
  if (view === "history") renderHistory();
  if (view === "voices") renderVoiceGrid();
}

/* ---------- player ---------- */

async function play(path, title) {
  try {
    const url = await invoke("read_audio", { path });
    playUrl(url, title);
  } catch (e) { toast(String(e), true); }
}

function playUrl(url, title) {
  const audio = $("#audio");
  audio.src = url;
  $("#player-title").textContent = title;
  $("#player").classList.remove("hidden");
  audio.play().catch(() => {});
}

/* ---------- account ---------- */

async function refreshAccount() {
  if (!state.hasKey) { $("#quota").replaceChildren(); return; }
  try {
    const acc = await invoke("account");
    renderAccount(acc);
  } catch (e) {
    $("#quota").textContent = "Akun tidak bisa dicek.";
    $("#account-info").textContent = String(e);
  }
}

function renderAccount(acc) {
  const used = acc.character_count ?? 0;
  const limit = acc.character_limit ?? 0;
  const pct = limit ? Math.min(100, (used / limit) * 100) : 0;
  const reset = acc.next_character_count_reset_unix
    ? new Date(acc.next_character_count_reset_unix * 1000).toLocaleDateString("id-ID", { day: "numeric", month: "short" })
    : null;
  // Set through CSSOM: the CSP blocks inline style attributes.
  const fill = el("div");
  fill.style.width = `${pct}%`;
  $("#quota").replaceChildren(
    el("div", {}, `Paket: ${acc.tier ?? "?"}`),
    el("div", {}, `${fmt(used)} / ${fmt(limit)} karakter`),
    el("div", { class: "meter" }, fill),
    reset ? el("div", {}, `Reset ${reset}`) : null);
  $("#account-info").textContent =
    `Terhubung · paket ${acc.tier ?? "?"} · sisa ${fmt(Math.max(0, limit - used))} karakter` +
    (acc.can_use_instant_voice_cloning === false ? " · paket ini belum bisa voice cloning" : "");
}

/* ---------- studio ---------- */

function currentModel() {
  return state.models.find((m) => m.model_id === $("#model-select").value);
}

function updateCharInfo() {
  const n = [...$("#script").value].length;
  const limit = currentModel()?.maximum_text_length_per_request || 5000;
  const parts = Math.max(1, Math.ceil(n / limit));
  $("#char-info").textContent = n === 0
    ? `Batas ${fmt(limit)} karakter per bagian`
    : `${fmt(n)} karakter${parts > 1 ? ` · dibagi otomatis jadi ±${parts} bagian` : ""}`;
}

function insertAtCursor(snippet) {
  const area = $("#script");
  const start = area.selectionStart ?? area.value.length;
  const end = area.selectionEnd ?? start;
  const before = area.value.slice(0, start);
  const pad = before && !/\s$/.test(before) ? " " : "";
  area.value = `${before}${pad}${snippet} ${area.value.slice(end)}`;
  const pos = start + pad.length + snippet.length + 1;
  area.focus();
  area.setSelectionRange(pos, pos);
  updateCharInfo();
}

function renderTags() {
  $("#tag-bar").replaceChildren(...TAGS.map(([tag, label]) =>
    el("button", { class: "chip", title: `[${tag}]`, onclick: () => insertAtCursor(`[${tag}]`) }, label)));
}

function renderSliders(values) {
  $("#sliders").replaceChildren(...SLIDERS.map((s) => {
    const value = el("b", {}, Number(values[s.key]).toFixed(2));
    return el("div", { class: "field" },
      el("div", { class: "slider-head" }, el("span", {}, s.label), value),
      el("input", {
        type: "range", id: `s-${s.key}`, min: s.min, max: s.max, step: s.step, value: values[s.key],
        oninput: (e) => { value.textContent = Number(e.target.value).toFixed(2); },
      }),
      el("div", { class: "slider-ends" }, el("span", {}, s.ends[0]), el("span", {}, s.ends[1])));
  }));
  $("#speaker-boost").checked = !!values.use_speaker_boost;
}

function readVoiceSettings() {
  const out = { use_speaker_boost: $("#speaker-boost").checked };
  for (const s of SLIDERS) out[s.key] = Number($(`#s-${s.key}`).value);
  return out;
}

function renderModels() {
  const select = $("#model-select");
  const saved = state.settings.model_id;
  const models = [...state.models].sort((a, b) => modelRank(a.model_id) - modelRank(b.model_id));
  select.replaceChildren(...models.map((m) => el("option", { value: m.model_id }, m.name || m.model_id)));
  if (models.some((m) => m.model_id === saved)) select.value = saved;
  renderLanguages();
  updateCharInfo();
}

/** Newest expressive models first, so v4 is the default when the account has it. */
function modelRank(id) {
  const order = ["eleven_v4", "eleven_v4_turbo", "eleven_v3", "eleven_multilingual_v2", "eleven_flash_v2_5", "eleven_turbo_v2_5"];
  const i = order.indexOf(id);
  return i === -1 ? order.length : i;
}

function renderLanguages() {
  const model = currentModel();
  const select = $("#language-select");
  const langs = [...(model?.languages || [])].sort((a, b) => a.name.localeCompare(b.name));
  const multilingualV2 = (model?.model_id || "").startsWith("eleven_multilingual_v2");
  select.replaceChildren(
    el("option", { value: "" }, "Otomatis (deteksi dari teks)"),
    ...(multilingualV2 ? [] : langs.map((l) => el("option", { value: l.language_id }, l.name))));
  const saved = state.settings.language_code;
  if ([...select.options].some((o) => o.value === saved)) select.value = saved;
  $("#model-note").textContent = model?.description ? model.description.slice(0, 140) : "";
}

function voiceLabel(v) {
  const bits = ["accent", "gender", "age"].map((k) => v.labels?.[k]).filter(Boolean);
  return bits.length ? `${v.name} · ${bits.join(", ")}` : v.name;
}

function renderVoiceSelect() {
  const select = $("#voice-select");
  const groups = new Map();
  for (const v of state.voices) {
    const key = CATEGORY_NAMES[v.category] || v.category || "Lainnya";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(v);
  }
  select.replaceChildren(...[...groups].map(([name, voices]) =>
    el("optgroup", { label: name }, voices.map((v) => el("option", { value: v.voice_id }, voiceLabel(v))))));

  let chosen = state.voices.find((v) => v.voice_id === state.settings.voice_id);
  if (!chosen) chosen = state.voices.find((v) => (v.name || "").toLowerCase().startsWith(PREFERRED_VOICE));
  if (!chosen) chosen = state.voices.find((v) => v.category === "premade") || state.voices[0];
  if (chosen) select.value = chosen.voice_id;

  const hasFin = state.voices.some((v) => (v.name || "").toLowerCase().startsWith(PREFERRED_VOICE));
  $("#voice-note").textContent = state.voices.length && !hasFin
    ? "Suara Fin tidak ada di akunmu. Tambahkan dari Voice Library ElevenLabs, lalu muat ulang daftar suara."
    : "";
}

function selectedVoice() {
  return state.voices.find((v) => v.voice_id === $("#voice-select").value);
}

async function previewVoice(voice) {
  if (!voice?.preview_url) { toast("Suara ini tidak punya contoh audio.", true); return; }
  try {
    const url = await invoke("voice_preview", { url: voice.preview_url });
    playUrl(url, `Contoh: ${voice.name}`);
  } catch (e) { toast(String(e), true); }
}

async function generate() {
  const voice = selectedVoice();
  const model = currentModel();
  const text = $("#script").value.trim();
  if (!state.hasKey) { toast("Masukkan API key dulu di Pengaturan.", true); show("settings"); return; }
  if (!text) { toast("Tulis naskahnya dulu.", true); return; }
  if (!voice || !model) { toast("Daftar suara/model belum termuat.", true); return; }
  const seedRaw = $("#seed").value.trim();
  const request = {
    text,
    voice_id: voice.voice_id,
    voice_name: voice.name || voice.voice_id,
    model_id: model.model_id,
    voice_settings: readVoiceSettings(),
    language_code: $("#language-select").value,
    seed: seedRaw === "" ? null : Number(seedRaw),
    output_format: $("#format-select").value,
    max_chars: model.maximum_text_length_per_request || null,
  };
  const box = $("#result");
  try {
    const item = await busy("Membuat suara…", () => invoke("generate", { request }));
    state.history.unshift(item);
    Object.assign(state.settings, {
      model_id: request.model_id, voice_id: request.voice_id, voice_name: request.voice_name,
      voice_settings: request.voice_settings, language_code: request.language_code, output_format: request.output_format,
    });
    box.className = "card result";
    box.replaceChildren(
      el("h3", {}, "Selesai ✓"),
      el("p", { class: "muted" }, `${item.voice_name} · ${item.model_id} · ${fmt(item.characters)} karakter${item.parts > 1 ? ` · ${item.parts} bagian` : ""}`),
      el("div", { class: "row" },
        el("button", { class: "btn primary small", onclick: () => play(item.path, item.text) }, "▶ Putar"),
        el("button", { class: "btn small", onclick: () => invoke("reveal_path", { path: item.path }) }, "📂 Tampilkan di folder")),
      el("p", { class: "muted" }, item.path));
    play(item.path, item.text);
    refreshAccount();
  } catch (e) {
    box.className = "card result err";
    box.replaceChildren(el("h3", {}, "Gagal"), el("p", {}, String(e)));
  }
}

/* ---------- voices page ---------- */

async function loadVoices() {
  if (!state.hasKey) return;
  try {
    const [models, voices] = await Promise.all([invoke("list_models"), invoke("list_voices", { category: null })]);
    state.models = models;
    state.voices = voices.sort((a, b) => (a.name || "").localeCompare(b.name || ""));
    renderModels();
    renderVoiceSelect();
    renderVoiceGrid();
  } catch (e) { toast(String(e), true); }
}

function renderVoiceGrid() {
  const query = $("#voice-search").value.trim().toLowerCase();
  const list = state.voices.filter((v) => {
    if (state.voiceFilter && v.category !== state.voiceFilter) return false;
    if (!query) return true;
    const hay = [v.name, v.description, v.category, ...Object.values(v.labels || {})].join(" ").toLowerCase();
    return hay.includes(query);
  });
  const grid = $("#voice-grid");
  if (!state.hasKey) { grid.replaceChildren(el("p", { class: "muted" }, "Masukkan API key dulu untuk melihat suara.")); return; }
  if (!list.length) { grid.replaceChildren(el("p", { class: "muted" }, "Tidak ada suara yang cocok.")); return; }
  const current = $("#voice-select").value;
  grid.replaceChildren(...list.map((v) => el("div", { class: v.voice_id === current ? "voice selected" : "voice" },
    el("div", { class: "name" }, v.name || v.voice_id),
    el("div", {}, el("span", { class: "badge" }, CATEGORY_NAMES[v.category] || v.category || "?"),
      ...["accent", "gender", "age", "use_case", "descriptive"].map((k) => v.labels?.[k]).filter(Boolean).map((t) => el("span", { class: "badge" }, t))),
    v.description ? el("div", { class: "desc" }, v.description) : null,
    el("div", { class: "row" },
      el("button", { class: "btn small", onclick: () => previewVoice(v) }, "▶ Contoh"),
      el("button", { class: "btn small primary", onclick: () => useVoice(v.voice_id) }, "Pakai di Studio")))));
}

function useVoice(id) {
  $("#voice-select").value = id;
  show("studio");
  toast(`Suara dipilih: ${selectedVoice()?.name}`);
}

async function pickCloneFiles() {
  try {
    const picked = await tauri.dialog.open({
      multiple: true,
      filters: [{ name: "Rekaman suara", extensions: ["mp3", "wav", "m4a", "flac", "ogg", "webm"] }],
    });
    if (!picked) return;
    state.cloneFiles = Array.isArray(picked) ? picked : [picked];
    $("#clone-files").textContent = state.cloneFiles.map((p) => p.split(/[\\/]/).pop()).join(", ");
  } catch (e) { toast(String(e), true); }
}

async function createClone() {
  const name = $("#clone-name").value.trim();
  if (!name) { toast("Beri nama suaranya dulu.", true); return; }
  if (!state.cloneFiles.length) { toast("Pilih rekaman suaranya dulu.", true); return; }
  if (!$("#clone-consent").checked) { toast("Centang pernyataan izin suara dulu.", true); return; }
  try {
    await busy("Mengunggah dan membuat kloning…", () => invoke("clone_voice", {
      name,
      description: $("#clone-desc").value,
      files: state.cloneFiles,
      removeBackgroundNoise: $("#clone-denoise").checked,
      consent: true,
    }));
    toast(`Suara "${name}" berhasil dibuat!`);
    state.cloneFiles = [];
    $("#clone-files").textContent = "Belum ada file dipilih.";
    $("#clone-consent").checked = false;
    await loadVoices();
    const created = state.voices.find((v) => v.name === name && v.category === "cloned");
    if (created) useVoice(created.voice_id);
  } catch (e) { toast(String(e), true); }
}

/* ---------- history ---------- */

function renderHistory() {
  const list = $("#history-list");
  if (!state.history.length) {
    list.replaceChildren(el("li", { class: "muted" }, "Belum ada audio yang dibuat."));
    return;
  }
  list.replaceChildren(...state.history.map((h) => el("li", {},
    el("div", { class: "text" }, h.text),
    el("div", { class: "meta" },
      `${h.voice_name} · ${h.model_id} · ${new Date(h.created_ms).toLocaleString("id-ID")} · ${fmt(h.characters)} karakter`),
    el("div", { class: "row" },
      el("button", { class: "btn small primary", onclick: () => play(h.path, h.text) }, "▶ Putar"),
      el("button", { class: "btn small", onclick: () => invoke("reveal_path", { path: h.path }).catch((e) => toast(String(e), true)) }, "📂 Folder"),
      el("button", { class: "btn small", onclick: () => reuse(h) }, "↺ Pakai lagi"),
      el("button", { class: "btn small", onclick: () => forget(h.id) }, "Hapus dari riwayat")))));
}

function reuse(h) {
  $("#script").value = h.text;
  if (state.voices.some((v) => v.voice_id === h.voice_id)) $("#voice-select").value = h.voice_id;
  if (state.models.some((m) => m.model_id === h.model_id)) { $("#model-select").value = h.model_id; renderLanguages(); }
  $("#language-select").value = h.language_code || "";
  renderSliders(h.voice_settings);
  updateCharInfo();
  show("studio");
}

async function forget(id) {
  try {
    state.history = await invoke("forget_history", { id });
    renderHistory();
    toast("Dihapus dari riwayat. File audionya tetap ada di folder.");
  } catch (e) { toast(String(e), true); }
}

/* ---------- settings ---------- */

async function saveKey() {
  const key = $("#api-key").value.trim();
  if (!key) { toast("Tempel API key-nya dulu.", true); return; }
  try {
    const acc = await busy("Mengecek API key…", () => invoke("set_api_key", { key }));
    $("#api-key").value = "";
    state.hasKey = true;
    $("#key-banner").classList.add("hidden");
    renderAccount(acc);
    toast("API key tersimpan.");
    await loadVoices();
  } catch (e) { toast(String(e), true); }
}

async function clearKey() {
  try {
    await invoke("clear_api_key");
    state.hasKey = false;
    $("#key-banner").classList.remove("hidden");
    $("#account-info").textContent = "API key sudah dihapus dari komputer ini.";
    refreshAccount();
  } catch (e) { toast(String(e), true); }
}

async function changeOutput() {
  try {
    const dir = await tauri.dialog.open({ directory: true, defaultPath: state.settings.output_dir });
    if (!dir) return;
    state.settings = await invoke("set_output_dir", { dir });
    $("#output-dir").textContent = state.settings.output_dir;
  } catch (e) { toast(String(e), true); }
}

function openOutput() {
  invoke("open_folder", { path: state.settings.output_dir }).catch((e) => toast(String(e), true));
}

/* ---------- start ---------- */

async function init() {
  initTheme();
  renderTags();
  for (const b of document.querySelectorAll(".nav")) b.addEventListener("click", () => show(b.dataset.view));
  for (const b of document.querySelectorAll("[data-goto]")) b.addEventListener("click", () => show(b.dataset.goto));
  for (const t of document.querySelectorAll("#voice-tabs .tab")) {
    t.addEventListener("click", () => {
      state.voiceFilter = t.dataset.cat;
      for (const o of document.querySelectorAll("#voice-tabs .tab")) o.classList.toggle("active", o === t);
      renderVoiceGrid();
    });
  }
  $("#script").addEventListener("input", updateCharInfo);
  $("#custom-tag-btn").addEventListener("click", () => {
    const tag = $("#custom-tag").value.trim().replace(/^\[|\]$/g, "");
    if (tag) { insertAtCursor(`[${tag}]`); $("#custom-tag").value = ""; }
  });
  $("#model-select").addEventListener("change", () => { renderLanguages(); updateCharInfo(); });
  $("#voice-preview-btn").addEventListener("click", () => previewVoice(selectedVoice()));
  $("#generate-btn").addEventListener("click", generate);
  $("#reset-settings").addEventListener("click", () => renderSliders(DEFAULT_VOICE_SETTINGS));
  $("#voice-search").addEventListener("input", renderVoiceGrid);
  $("#refresh-voices").addEventListener("click", () => busy("Memuat suara…", loadVoices));
  $("#clone-pick").addEventListener("click", pickCloneFiles);
  $("#clone-btn").addEventListener("click", createClone);
  $("#save-key").addEventListener("click", saveKey);
  $("#clear-key").addEventListener("click", clearKey);
  $("#get-key").addEventListener("click", () => invoke("open_elevenlabs", { path: "app/settings/api-keys" }));
  $("#change-output").addEventListener("click", changeOutput);
  $("#open-output").addEventListener("click", openOutput);
  $("#open-output-2").addEventListener("click", openOutput);

  let snapshot;
  try {
    snapshot = await invoke("get_state");
  } catch {
    snapshot = { has_key: false, settings: { voice_settings: DEFAULT_VOICE_SETTINGS, output_format: "mp3_44100_128", output_dir: "" }, history: [] };
  }
  state.hasKey = snapshot.has_key;
  state.settings = snapshot.settings;
  state.history = snapshot.history;
  renderSliders(state.settings.voice_settings || DEFAULT_VOICE_SETTINGS);
  $("#format-select").value = state.settings.output_format || "mp3_44100_128";
  $("#output-dir").textContent = state.settings.output_dir;
  updateCharInfo();

  if (!state.hasKey) {
    $("#key-banner").classList.remove("hidden");
    show("settings");
    return;
  }
  await busy("Memuat suara dan model…", loadVoices);
  refreshAccount();
}

init();
