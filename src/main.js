const tauri = window.__TAURI_INTERNALS__;
const invoke = (command, args = {}) => tauri.invoke(command, args);
const listen = (event, handler) =>
  tauri.invoke("plugin:event|listen", { event, target: { kind: "Any" }, handler: tauri.transformCallback(handler) });

const STATUS_LABELS = {
  starting: "carregando",
  ok: "rodando",
  offline: "sem internet",
  siteDown: "site fora do ar",
  frozen: "sem resposta",
  recovering: "recuperando",
  stopped: "parada",
};

const ICONS = {
  plus: ["...##...", "...##...", "...##...", "########", "########", "...##...", "...##...", "...##..."],
  close: ["##....##", "###..###", ".######.", "..####..", "..####..", ".######.", "###..###", "##....##"],
  play: [".#......", ".##.....", ".###....", ".####...", ".####...", ".###....", ".##.....", ".#......"],
  stop: ["........", ".######.", ".######.", ".######.", ".######.", ".######.", ".######.", "........"],
  gear: [".#.##.#.", "########", ".##..##.", "###..###", "###..###", ".##..##.", "########", ".#.##.#."],
  reload: ["..####.#", ".#....##", "#....###", "#.......", "#.......", "#......#", ".#....#.", "..####.."],
  home: ["...##...", "..####..", ".######.", "########", ".#....#.", ".#.##.#.", ".#.##.#.", ".######."],
  key: ["........", ".###....", "#...#...", "#...####", "#...#.##", ".###..#.", "........", "........"],
  popout: [".....###", "......##", ".....#.#", "###.#...", "#..#....", "#.......", "#......#", "########"],
  focus: ["########", "#......#", "#.####.#", "#.####.#", "#.####.#", "#.####.#", "#......#", "########"],
  main: ["#####.##", "#####.##", "#####...", "#####.##", "#####.##", "#####...", "#####.##", "#####.##"],
  grid: ["###.###.", "###.###.", "###.###.", "........", "###.###.", "###.###.", "###.###.", "........"],
  panel: ["......##", "......##", "...##.##", "...##.##", "##.##.##", "##.##.##", "##.##.##", "########"],
  lock: ["..####..", ".#....#.", ".#....#.", "########", "###..###", "###..###", "########", "########"],
  bell: ["...##...", "..####..", ".######.", ".######.", ".######.", "########", "........", "...##..."],
  eye: ["........", "..####..", ".#....#.", "#..##..#", "#..##..#", ".#....#.", "..####..", "........"],
  chip: [".#.#.#..", "#######.", ".#...#..", "##.#.##.", ".#...#..", "#######.", ".#.#.#..", "........"],
  heart: [".##.##..", "#######.", "#######.", "#######.", ".#####..", "..###...", "...#....", "........"],
  bolt: ["....##..", "...##...", "..##....", ".######.", "....##..", "...##...", "..##....", ".##....."],
  user: ["..####..", "..####..", "..####..", "...##...", ".######.", "########", "########", "########"],
  warn: ["...##...", "...##...", "..####..", "..#..#..", ".##..##.", ".######.", "###..###", "########"],
  layers: ["...##...", ".##..##.", "##....##", ".##..##.", "#.####.#", ".##..##.", "..####..", "........"],
  swap: ["..#.....", ".##.....", "########", ".##.....", "..#..#..", ".....##.", "########", ".....##."],
};

const BADGE_COLORS = ["#ffc93c", "#ffe28a", "#ffad1f", "#f6f3ea", "#e8b33a", "#ffd56b"];

const $ = (id) => document.getElementById(id);
const viewportEl = $("viewport");
const statusEl = $("status");

let state = {
  games: [],
  active: null,
  settings: { view: "focus", watchWords: [] },
  covered: false,
  popped: [],
  tiles: [],
  vault: { mode: "device", unlocked: false, logins: [] },
};
let overview = { health: {}, history: [], playtime: {} };
let page = null;
let mainKey = "";
let pendingRemoval = null;
let pendingTimer = 0;
let statusTimer = 0;
let audio = null;

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children.filter((child) => child !== null && child !== undefined && child !== false));
  return node;
}

function svg(tag, attrs = {}, children = []) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  Object.entries(attrs).forEach(([key, value]) => node.setAttribute(key, value));
  node.append(...children);
  return node;
}

function icon(name, size = 14) {
  const rows = ICONS[name] ?? ICONS.close;
  const rects = [];
  rows.forEach((row, y) => {
    let start = -1;
    for (let x = 0; x <= row.length; x++) {
      const filled = row[x] === "#";
      if (filled && start < 0) start = x;
      if (!filled && start >= 0) {
        rects.push(svg("rect", { x: start, y, width: x - start, height: 1 }));
        start = -1;
      }
    }
  });
  return svg("svg", { class: "px-icon", viewBox: `0 0 ${rows[0].length} ${rows.length}`, width: size, height: size, fill: "currentColor" }, rects);
}

function hydrateIcons(root = document) {
  root.querySelectorAll("[data-icon]").forEach((node) => {
    if (node.querySelector(".px-icon")) return;
    node.prepend(icon(node.dataset.icon, node.classList.contains("boxed") ? 15 : 14));
  });
}

function setStatus(text, kind = "ok", iconName = null) {
  const icons = { ok: "heart", error: "warn", alert: "bell" };
  statusEl.replaceChildren(el("div", { className: `toast ${kind}` }, [icon(iconName ?? icons[kind] ?? "bell", 12), el("span", { textContent: text })]));
  clearTimeout(statusTimer);
  statusTimer = setTimeout(() => statusEl.replaceChildren(), 7000);
}

async function call(command, args = {}) {
  try {
    state = await invoke(command, args);
    render();
    return true;
  } catch (error) {
    setStatus(String(error), "error");
    render();
    return false;
  }
}

function allSessions() {
  return state.games.flatMap((game) => game.sessions.map((session) => ({ game, session })));
}

function findSession(id) {
  return allSessions().find((entry) => entry.session.id === id) ?? null;
}

function statusOf(session) {
  if (!session.running) return "stopped";
  return overview.health[session.id]?.status ?? "starting";
}

function playtimeOf(session) {
  return overview.playtime?.[session.id] ?? session.playtimeSecs ?? 0;
}

function gamePlaytime(game) {
  return game.sessions.reduce((sum, session) => sum + playtimeOf(session), 0);
}

function formatHours(seconds) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h) return `${h}h ${String(m).padStart(2, "0")}min`;
  return `${m}min`;
}

function groupNames() {
  return [...new Set(allSessions().map((entry) => entry.session.group).filter(Boolean))].sort();
}

function badgeColor(text) {
  let hash = 0;
  for (const char of text) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return BADGE_COLORS[hash % BADGE_COLORS.length];
}

const gameIcons = new Map();

function gameIcon(url) {
  if (!gameIcons.has(url)) gameIcons.set(url, invoke("game_icon", { url }).catch(() => null));
  return gameIcons.get(url);
}

function gameBadge(game, extraStyle = "") {
  const badge = el("span", {
    className: "badge",
    textContent: game.name.trim().charAt(0).toUpperCase() || "?",
    style: `background:${badgeColor(game.name)};${extraStyle}`,
  });
  gameIcon(game.url).then((src) => {
    if (!src) return;
    const img = el("img", { src, alt: "", draggable: false });
    img.onload = () => {
      badge.classList.add("has-icon");
      badge.replaceChildren(img);
    };
  });
  return badge;
}

function dot(status) {
  return el("span", { className: `dot ${status}`, title: STATUS_LABELS[status] });
}

function iconButton(name, title, onClick, extraClass = "") {
  return el(
    "button",
    {
      className: `icon-btn ${extraClass}`.trim(),
      title,
      onclick: (event) => {
        event.stopPropagation();
        onClick();
      },
    },
    [icon(name)],
  );
}

function button(label, onClick, { kind = "", iconName = null, small = false, type = "button" } = {}) {
  return el("button", { type, className: `btn ${kind} ${small ? "small" : ""}`.trim(), onclick: onClick }, [
    iconName ? icon(iconName, small ? 12 : 14) : null,
    label,
  ]);
}

function removeButton(key, title, onConfirm, extraClass = "") {
  const armed = pendingRemoval === key;
  const node = el("button", {
    className: armed ? "icon-btn armed" : `icon-btn danger ${extraClass}`.trim(),
    title,
    onclick: (event) => {
      event.stopPropagation();
      if (pendingRemoval === key) {
        pendingRemoval = null;
        onConfirm();
        return;
      }
      pendingRemoval = key;
      clearTimeout(pendingTimer);
      pendingTimer = setTimeout(() => {
        pendingRemoval = null;
        renderSidebar();
      }, 3000);
      renderSidebar();
    },
  });
  node.append(armed ? "Remover?" : icon("close", 12));
  return node;
}

function field(label, control) {
  return el("label", { className: "field" }, [label, control]);
}

function checkbox(label, checked, onChange, title = "") {
  return el("label", { className: "check", title }, [
    el("input", { type: "checkbox", checked, onchange: (event) => onChange(event.target.checked) }),
    el("span", { textContent: label }),
  ]);
}

function heading(text, iconName) {
  return el("h3", {}, [icon(iconName, 12), text]);
}

function openPage(next) {
  page = next;
  mainKey = "";
  call("set_covered", { covered: true });
}

function closePage() {
  page = null;
  mainKey = "";
  call("set_covered", { covered: false });
}

function refreshMain() {
  mainKey = "";
  renderMain();
}

function selectSession(id) {
  page = null;
  mainKey = "";
  return call("select_session", { sessionId: id });
}

function render() {
  renderSidebar();
  renderToolbar();
  renderTiles();
  renderMain();
}

function renderSidebar() {
  const groups = groupNames();
  $("groups").replaceChildren(
    ...(groups.length ? [el("div", { className: "section-title", textContent: "Grupos" }), ...groups.map(renderGroup)] : []),
  );
  $("games").replaceChildren(
    ...(state.games.length
      ? [el("div", { className: "section-title", textContent: "Jogos" }), ...state.games.map(renderGame)]
      : [el("p", { className: "hint center-text", textContent: "Nenhum jogo ainda. Adicione o primeiro abaixo." })]),
  );
}

function renderGroup(name) {
  const sessions = allSessions().map((entry) => entry.session);
  const members = sessions.filter((s) => s.group === name);
  const running = members.filter((s) => s.running).length;
  const exclusive = running === members.length && sessions.filter((s) => s.running).length === running;
  return el("div", { className: "group-row" }, [
    el("span", { className: "chip" }, [name]),
    el("span", { className: "name" }),
    el("span", { className: "count", textContent: `${running}/${members.length}` }),
    exclusive
      ? el("span", { className: "chip portal", textContent: "ativo" })
      : button("Trocar", () => call("switch_group", { group: name }), { small: true, iconName: "swap" }),
  ]);
}

function renderGame(game) {
  const running = game.sessions.filter((s) => s.running).length;
  return el("div", { className: "game" }, [
    el("div", { className: "game-header" }, [
      gameBadge(game),
      el("div", { className: "game-info", title: game.url }, [
        el("span", { className: "game-name", textContent: game.name }),
        el("span", { className: "game-meta", textContent: `${running}/${game.sessions.length} ativas · ${formatHours(gamePlaytime(game))}`, title: `${running} de ${game.sessions.length} contas rodando · ${formatHours(gamePlaytime(game))} jogadas no total` }),
      ]),
      iconButton("plus", "Nova sessão (outra conta)", () => {
        page = null;
        mainKey = "";
        call("add_session", { gameId: game.id });
      }),
      removeButton(`game:${game.id}`, "Remover jogo e todas as sessões", () => call("remove_game", { gameId: game.id })),
    ]),
    el("div", { className: "sessions" }, game.sessions.map(renderSession)),
  ]);
}

function renderSession(session) {
  const status = statusOf(session);
  const popped = state.popped.includes(session.id);
  const classes = ["session"];
  if (session.id === state.active) classes.push("active");
  if (!session.running) classes.push("stopped");
  return el(
    "div",
    {
      className: classes.join(" "),
      title: `${STATUS_LABELS[status]} · ${formatHours(playtimeOf(session))} online no total${popped ? " · aberta em outra janela" : ""}`,
      onclick: () => selectSession(session.id),
    },
    [
      dot(status),
      el("span", { className: "session-name", textContent: session.name }),
      session.group ? el("span", { className: "chip", textContent: session.group, title: `Grupo: ${session.group}` }) : null,
      popped ? el("span", { className: "chip portal", title: "Aberta em outra janela" }, [icon("popout", 10)]) : null,
      iconButton(
        session.running ? "stop" : "play",
        session.running ? "Parar esta conta" : "Iniciar esta conta",
        () => call("set_running", { sessionId: session.id, running: !session.running }),
        "hover-only",
      ),
      iconButton("gear", "Configurar sessão e login", () => openPage({ type: "session", id: session.id }), "hover-only"),
      removeButton(`session:${session.id}`, "Remover sessão (apaga o login dela)", () => call("remove_session", { sessionId: session.id }), "hover-only"),
    ],
  );
}

function renderToolbar() {
  const current = findSession(state.active);
  const titles = { add: ["plus", "Adicionar jogo"], settings: ["gear", "Configurações"] };
  if (page && titles[page.type]) {
    const [iconName, text] = titles[page.type];
    $("title").replaceChildren(icon(iconName, 14), el("strong", { textContent: text }));
  } else if (state.settings.view === "panel" && !page) {
    $("title").replaceChildren(icon("panel", 14), el("strong", { textContent: "Painel" }));
  } else if (current) {
    $("title").replaceChildren(
      dot(statusOf(current.session)),
      el("span", { className: "crumb", textContent: current.game.name }),
      el("span", { className: "crumb-sep", textContent: "›" }),
      el("strong", { textContent: current.session.name }),
    );
  } else {
    $("title").replaceChildren(el("strong", { textContent: "IdlePort" }));
  }

  document.querySelectorAll("#views button").forEach((node) => {
    node.classList.toggle("on", !page && node.dataset.view === state.settings.view);
  });

  const showActions = Boolean(current && current.session.running && !page && state.settings.view !== "panel");
  $("actions").hidden = !showActions;
  $("fill").disabled = !current || !state.vault.logins.includes(current.session.id);
}

function renderTiles() {
  $("tiles").replaceChildren(
    ...state.tiles
      .map((tile) => {
        const found = findSession(tile.sessionId);
        if (!found) return null;
        const status = statusOf(found.session);
        return el(
          "div",
          {
            className: `tile-label${tile.sessionId === state.active ? " active" : ""}`,
            style: `left:${tile.x}px;top:${tile.y}px;width:${tile.width}px;height:${tile.height}px`,
            title: STATUS_LABELS[status],
            onclick: () => selectSession(tile.sessionId),
          },
          [
            dot(status),
            el("span", { className: "session-name", textContent: `${found.game.name} › ${found.session.name}` }),
            iconButton("focus", "Abrir em foco", async () => {
              if (await selectSession(tile.sessionId)) call("set_view", { view: "focus" });
            }),
            iconButton("reload", "Recarregar", () => call("reload_session", { sessionId: tile.sessionId })),
          ],
        );
      })
      .filter(Boolean),
  );
}

function currentMainKey() {
  if (page) return `page:${page.type}:${page.id ?? ""}`;
  const view = state.settings.view;
  if (view === "panel") return "panel";
  if (!state.games.length) return "welcome";
  if (view === "focus") {
    if (!state.active) return "pick";
    if (state.popped.includes(state.active)) return "popped";
    const current = findSession(state.active);
    if (!current) return "pick";
    if (!current.session.running) return `stopped:${state.active}`;
    return "none";
  }
  const visible = allSessions().filter((e) => e.session.running && !state.popped.includes(e.session.id));
  return visible.length ? "none" : "idle";
}

function renderMain(force = false) {
  const key = currentMainKey();
  if (!force && key === mainKey) return;
  mainKey = key;
  viewportEl.replaceChildren(...buildMain(key));
}

function buildMain(key) {
  if (key.startsWith("page:add")) return buildAddPage(true);
  if (key.startsWith("page:settings")) return buildSettingsPage();
  if (key.startsWith("page:session")) return buildSessionPage(page.id);
  if (key === "panel") return buildPanel();
  if (key === "welcome") return buildWelcome();
  if (key === "pick") return [message("layers", "Escolha uma conta", "Clique numa conta na barra lateral para abrir o jogo aqui.")];
  if (key === "popped") return [message("popout", "Conta em outra janela", "Feche aquela janela para trazer a conta de volta para cá.")];
  if (key === "idle") return [message("play", "Nenhuma conta rodando", "Use ▶ na barra lateral ou troque para um grupo.")];
  if (key.startsWith("stopped:")) {
    const id = key.slice("stopped:".length);
    return [
      message("stop", "Esta conta está parada", "Ela não está gastando memória nem rodando o jogo.", [
        button("Iniciar conta", () => call("set_running", { sessionId: id, running: true }), { kind: "primary", iconName: "play" }),
      ]),
    ];
  }
  return [];
}

function message(iconName, title, hint, actions = []) {
  return el("div", { className: "center" }, [
    el("div", { className: "card narrow" }, [
      el("div", { className: "row middle" }, [el("span", { className: "stat-icon" }, [icon(iconName, 18)]), el("h2", { textContent: title })]),
      el("p", { className: "hint", textContent: hint }),
      actions.length ? el("div", { className: "row" }, actions) : null,
    ]),
  ]);
}

function addGameForm(asPage) {
  const form = el("form", { className: "list" }, [
    field("Nome do jogo", el("input", { name: "name", placeholder: "Ex.: Grand Line Idle", autocomplete: "off" })),
    field("Endereço do site", el("input", { name: "url", placeholder: "https://...", required: true, autocomplete: "off" })),
    el("div", { className: "page-actions" }, [
      asPage ? button("Cancelar", closePage, { kind: "ghost" }) : el("span"),
      button("Adicionar e abrir", null, { kind: "primary", iconName: "plus", type: "submit" }),
    ]),
  ]);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const data = new FormData(form);
    page = null;
    mainKey = "";
    call("add_game", { name: data.get("name"), url: data.get("url") });
  });
  requestAnimationFrame(() => form.elements.namedItem("url")?.focus());
  return form;
}

function buildWelcome() {
  return [
    el("div", { className: "center" }, [
      el("div", { className: "card narrow" }, [
        el("div", { className: "hero" }, [
          el("img", { src: "assets/hero.png", alt: "" }),
          el("h2", { textContent: "Bem-vindo ao IdlePort" }),
          el("p", { className: "hint", textContent: "Seus jogos idle, várias contas, tudo rodando 24h num lugar só." }),
        ]),
        addGameForm(false),
        el("p", { className: "hint", textContent: "Depois de logar, use o + ao lado do jogo para abrir outra conta. Cada conta tem cookies, login e cache separados." }),
      ]),
    ]),
  ];
}

function buildAddPage() {
  return [
    el("div", { className: "center" }, [
      el("div", { className: "card narrow" }, [
        el("h2", { textContent: "Adicionar jogo" }),
        el("p", { className: "hint", textContent: "Cole o endereço da página do jogo. A primeira conta abre na hora." }),
        addGameForm(true),
      ]),
    ]),
  ];
}

function unlockForm(onDone) {
  const input = el("input", { type: "password", placeholder: "Senha mestra" });
  const submit = async () => {
    if (await call("vault_unlock", { password: input.value })) onDone();
  };
  input.addEventListener("keydown", (event) => event.key === "Enter" && submit());
  return el("div", { className: "row" }, [field("Senha mestra", input), button("Desbloquear", submit, { kind: "primary", iconName: "lock" })]);
}

function buildVaultSection() {
  const vault = state.vault;
  if (vault.mode === "master" && !vault.unlocked) {
    return [el("p", { className: "hint", textContent: "O cofre está bloqueado." }), unlockForm(refreshMain)];
  }
  const newPassword = el("input", { type: "password", placeholder: vault.mode === "master" ? "Nova senha mestra" : "Opcional" });
  return [
    el("p", {
      className: "hint",
      textContent:
        vault.mode === "master"
          ? "Os logins estão criptografados com a sua senha mestra. Ela é pedida uma vez a cada vez que o app abre."
          : "Os logins são criptografados (AES-256) com uma chave guardada no Gerenciador de Credenciais do Windows. Nada sai do seu PC.",
    }),
    el("div", { className: "row" }, [
      field(vault.mode === "master" ? "Trocar senha mestra" : "Senha mestra", newPassword),
      button(
        vault.mode === "master" ? "Trocar" : "Definir",
        async () => {
          if (!newPassword.value) return setStatus("Digite uma senha", "error");
          if (await call("vault_set_master", { password: newPassword.value })) {
            setStatus("Senha mestra definida");
            refreshMain();
          }
        },
        { iconName: "lock" },
      ),
    ]),
    vault.mode === "master"
      ? el("div", { className: "row" }, [
          button("Remover senha mestra", async () => (await call("vault_set_master", { password: null })) && refreshMain(), { kind: "ghost" }),
          button("Bloquear agora", async () => (await call("vault_lock")) && refreshMain(), { kind: "ghost", iconName: "lock" }),
        ])
      : null,
  ];
}

function buildSettingsPage() {
  const settings = structuredClone(state.settings);
  const words = el("div", { className: "list" });
  const renderWords = () =>
    words.replaceChildren(
      ...settings.watchWords.map((word, index) =>
        el("div", { className: "word-row" }, [
          el("input", { type: "text", value: word.text, placeholder: "ex.: verificação", oninput: (e) => (word.text = e.target.value) }),
          checkbox("recarregar", word.reload, (v) => (word.reload = v), "Recarrega a conta quando o texto aparecer"),
          iconButton("close", "Remover", () => {
            settings.watchWords.splice(index, 1);
            renderWords();
          }, "danger"),
        ]),
      ),
      settings.watchWords.length ? "" : el("p", { className: "hint", textContent: "Nenhum texto cadastrado." }),
    );
  renderWords();

  const text = (key, label, placeholder, type = "text") =>
    field(label, el("input", { type, value: settings[key] ?? "", placeholder, oninput: (e) => (settings[key] = e.target.value) }));
  const save = async () => {
    const ok = await call("update_settings", { settings });
    if (ok) setStatus("Configurações salvas");
    return ok;
  };

  const card = el("div", { className: "card" }, [
    el("h2", { textContent: "Configurações" }),
    heading("Geral", "gear"),
    checkbox("Fechar a janela manda o IdlePort para a bandeja (as contas continuam rodando)", settings.closeToTray, (v) => (settings.closeToTray = v)),
    heading("Alertas", "bell"),
    el("div", { className: "row middle" }, [
      checkbox("Notificação do Windows", settings.notify, (v) => (settings.notify = v)),
      checkbox("Tocar som", settings.sound, (v) => (settings.sound = v)),
    ]),
    text("discordWebhook", "Webhook do Discord", "https://discord.com/api/webhooks/..."),
    el("div", { className: "row" }, [
      text("telegramToken", "Token do bot do Telegram", "123456:ABC...", "password"),
      text("telegramChatId", "Chat ID do Telegram", "123456789"),
    ]),
    el("div", { className: "row" }, [button("Salvar e testar alertas", async () => (await save()) && call("test_alert"), { iconName: "bell" })]),
    heading("Textos monitorados", "eye"),
    el("p", { className: "hint", textContent: "O app avisa quando um destes textos aparece na tela de qualquer conta. Ele só lê, nunca clica nem digita." }),
    words,
    el("div", { className: "row" }, [
      button("Adicionar texto", () => {
        settings.watchWords.push({ text: "", reload: false });
        renderWords();
      }, { kind: "ghost", iconName: "plus", small: true }),
    ]),
    el("div", { className: "page-actions" }, [button("Fechar", closePage, { kind: "ghost" }), button("Salvar", save, { kind: "primary" })]),
    heading("Cofre de logins", "lock"),
    ...buildVaultSection(),
  ]);
  return [el("div", { className: "center" }, [card])];
}

function buildSessionPage(id) {
  const found = findSession(id);
  if (!found) {
    page = null;
    return buildMain(currentMainKey());
  }
  const { game, session } = found;
  const name = el("input", { value: session.name });
  const group = el("input", { value: session.group, placeholder: "ex.: Time A", list: "group-list" });
  const groupList = el("datalist", { id: "group-list" }, groupNames().map((g) => el("option", { value: g })));
  let autoRecover = session.autoRecover;
  const health = overview.health[id];
  const status = statusOf(session);

  const loginSection = () => {
    if (state.vault.mode === "master" && !state.vault.unlocked) {
      return [el("p", { className: "hint", textContent: "Desbloqueie o cofre para ver ou salvar o login desta conta." }), unlockForm(refreshMain)];
    }
    const saved = state.vault.logins.includes(id);
    const username = el("input", { placeholder: "E-mail ou usuário", autocomplete: "off" });
    const password = el("input", { type: "password", placeholder: saved ? "Digite para atualizar" : "Senha", autocomplete: "new-password" });
    const savedHint = el("p", { className: "hint", textContent: saved ? "Login salvo." : "Nenhum login salvo para esta conta." });
    if (saved) {
      invoke("vault_username", { sessionId: id })
        .then((value) => {
          if (value) {
            username.value = value;
            savedHint.textContent = `Login salvo para ${value}.`;
          }
        })
        .catch(() => {});
    }
    return [
      savedHint,
      el("div", { className: "row" }, [field("Usuário", username), field("Senha", password)]),
      el("div", { className: "row" }, [
        button("Salvar login", async () => {
          if (!password.value) return setStatus("Digite a senha", "error");
          if (await call("vault_save_login", { sessionId: id, username: username.value, password: password.value })) {
            setStatus("Login salvo no cofre");
            refreshMain();
          }
        }, { kind: "primary", iconName: "lock" }),
        saved
          ? button("Preencher agora", async () => {
              if (await selectSession(id)) call("vault_fill", { sessionId: id });
            }, { iconName: "key" })
          : null,
        saved ? button("Esquecer", async () => (await call("vault_forget", { sessionId: id })) && refreshMain(), { kind: "danger" }) : null,
      ]),
      el("p", { className: "hint", textContent: "\"Preencher\" só digita nos campos de usuário e senha. Nunca clica em entrar nem envia nada sozinho." }),
    ];
  };

  const card = el("div", { className: "card" }, [
    el("div", { className: "row middle" }, [
      gameBadge(game),
      el("h2", { textContent: `${game.name} › ${session.name}` }),
    ]),
    el("div", { className: "status-line" }, [
      dot(status),
      session.running ? `${STATUS_LABELS[status]}${health ? ` · ${health.reloads} recarga(s) automática(s)` : ""}` : "Conta parada",
      ` · ${formatHours(playtimeOf(session))} online no total`,
    ]),
    heading("Sessão", "user"),
    el("div", { className: "row" }, [field("Nome", name), field("Grupo", group), groupList]),
    checkbox("Recuperar sozinha (recarrega se travar, cair ou ficar sem internet)", autoRecover, (v) => (autoRecover = v)),
    el("div", { className: "page-actions" }, [
      el("div", { className: "row" }, [
        button("Fechar", closePage, { kind: "ghost" }),
        button(session.running ? "Parar" : "Iniciar", async () => (await call("set_running", { sessionId: id, running: !session.running })) && refreshMain(), {
          iconName: session.running ? "stop" : "play",
        }),
        button("Outra janela", async () => {
          page = null;
          mainKey = "";
          await call("pop_out", { sessionId: id });
          call("set_covered", { covered: false });
        }, { iconName: "popout" }),
      ]),
      button("Salvar", async () => {
        if (await call("update_session", { sessionId: id, name: name.value, group: group.value, autoRecover })) {
          setStatus("Sessão salva");
          refreshMain();
        }
      }, { kind: "primary" }),
    ]),
    heading("Login (cofre local)", "key"),
    ...loginSection(),
  ]);
  return [el("div", { className: "center" }, [card])];
}

function formatDuration(seconds) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return h ? `${h}h ${m}min` : `${m}min`;
}

function bars(samples) {
  if (!samples || samples.length < 2) return el("span", { className: "hint", textContent: "—" });
  const recent = samples.slice(-30);
  const max = Math.max(...recent);
  const min = Math.min(...recent);
  const span = Math.max(1, max - min);
  const width = recent.length * 4;
  return svg(
    "svg",
    { class: "bars", viewBox: `0 0 ${width} 24`, width, height: 24 },
    recent.map((value, i) => {
      const height = 4 + Math.round(((value - min) / span) * 5) * 4;
      return svg("rect", { x: i * 4, y: 24 - height, width: 3, height, class: i === recent.length - 1 ? "last" : "" });
    }),
  );
}

function progress(share) {
  const segments = 20;
  const filled = Math.round(share * segments);
  return el("div", { className: "progress", title: `${Math.round(share * 100)}%` }, [
    ...Array.from({ length: segments }, (_, i) => el("span", { className: i < filled ? "on" : "" })),
    el("em", { textContent: `${Math.round(share * 100)}%` }),
  ]);
}

function stat(iconName, label, value, color) {
  return el("div", { className: "stat" }, [
    el("span", { className: "stat-icon", style: `color:${color}` }, [icon(iconName, 18)]),
    el("div", {}, [el("div", { className: "label", textContent: label }), el("div", { className: "value", textContent: value })]),
  ]);
}

function buildPanel() {
  const entries = allSessions();
  const running = entries.filter((e) => e.session.running);
  const healths = running.map((e) => overview.health[e.session.id]).filter(Boolean);
  const memory = healths.reduce((sum, h) => sum + h.memoryMb, 0);
  const cpu = healths.reduce((sum, h) => sum + h.cpu, 0);
  const reloads = healths.reduce((sum, h) => sum + h.reloads, 0);
  const problems = running.filter((e) => ["offline", "siteDown", "frozen"].includes(statusOf(e.session))).length;
  const totalPlaytime = entries.reduce((sum, e) => sum + playtimeOf(e.session), 0);

  const gameRows = state.games
    .map((game) => ({ game, seconds: gamePlaytime(game) }))
    .sort((a, b) => b.seconds - a.seconds)
    .map(({ game, seconds }) => {
      const share = totalPlaytime ? seconds / totalPlaytime : 0;
      return el("tr", {}, [
        el("td", {}, [
          el("div", { className: "cell-status" }, [
            gameBadge(game, "width:22px;height:22px;font-size:12px"),
            el("span", { className: "account", textContent: game.name }),
          ]),
        ]),
        el("td", { textContent: `${game.sessions.length}` }),
        el("td", {}, [el("span", { className: "account", textContent: formatHours(seconds) })]),
        el("td", {}, [progress(share)]),
      ]);
    });

  const rows = entries.map(({ game, session }) => {
    const health = overview.health[session.id];
    const status = statusOf(session);
    return el("tr", {}, [
      el("td", {}, [el("div", { className: "cell-status" }, [dot(status), STATUS_LABELS[status]])]),
      el("td", {}, [el("span", { className: "account", textContent: `${game.name} › ${session.name}` })]),
      el("td", {}, [session.group ? el("span", { className: "chip", textContent: session.group }) : "—"]),
      el("td", { textContent: health ? `${health.memoryMb} MB` : "—" }),
      el("td", { textContent: health ? `${health.cpu.toFixed(1)}%` : "—" }),
      el("td", { textContent: health ? formatDuration(health.uptimeSecs) : "—" }),
      el("td", {}, [el("span", { className: "account", textContent: formatHours(playtimeOf(session)) })]),
      el("td", { textContent: health ? String(health.reloads) : "—" }),
      el("td", {}, [bars(health?.samples)]),
      el("td", {}, [
        el("div", { className: "row middle" }, [
          session.running ? button("Abrir", () => selectSession(session.id), { small: true, iconName: "focus" }) : null,
          button(session.running ? "Parar" : "Iniciar", () => call("set_running", { sessionId: session.id, running: !session.running }), {
            small: true,
            kind: "ghost",
            iconName: session.running ? "stop" : "play",
          }),
        ]),
      ]),
    ]);
  });

  const history = overview.history.length
    ? overview.history.map((event) =>
        el("div", { className: "history-row" }, [
          el("time", { textContent: new Date(event.ts * 1000).toLocaleTimeString("pt-BR") }),
          el("span", { className: "who", textContent: event.session }),
          el("span", { textContent: event.text }),
        ]),
      )
    : [el("div", { className: "empty", textContent: "Nada aconteceu ainda. Tudo tranquilo por aqui." })];

  return [
    el("div", { className: "panel" }, [
      el("div", { className: "stats" }, [
        stat("user", "Contas rodando", `${running.length}/${entries.length}`, "var(--ok)"),
        stat("heart", "Horas jogadas (total)", formatHours(totalPlaytime), "var(--slime)"),
        stat("chip", "Memória total", `${memory} MB`, "var(--slime-hi)"),
        stat("bolt", "CPU total", `${cpu.toFixed(1)}%`, "var(--slime)"),
        stat("reload", "Recargas automáticas", String(reloads), "var(--portal-2)"),
        stat("warn", "Com problema agora", String(problems), problems ? "var(--danger)" : "var(--muted)"),
      ]),
      el("div", { className: "section-title", textContent: "Horas por jogo" }),
      el("div", { className: "table-wrap" }, [
        el("table", {}, [
          el("thead", {}, [el("tr", {}, ["Jogo", "Contas", "Horas jogadas", "Parcela do total"].map((h) => el("th", { textContent: h })))]),
          el("tbody", {}, gameRows.length ? gameRows : [el("tr", {}, [el("td", { colSpan: 4, className: "empty", textContent: "Nenhum jogo ainda." })])]),
        ]),
      ]),
      el("div", { className: "section-title", textContent: "Contas" }),
      el("div", { className: "table-wrap" }, [
        el("table", {}, [
          el("thead", {}, [
            el("tr", {}, ["Status", "Conta", "Grupo", "Memória", "CPU", "Sessão atual", "Tempo total", "Recargas", "Memória (2,5 min)", ""].map((h) => el("th", { textContent: h }))),
          ]),
          el("tbody", {}, rows),
        ]),
      ]),
      el("div", { className: "section-title", textContent: "Histórico" }),
      el("div", { className: "history" }, history),
    ]),
  ];
}

function beep() {
  try {
    audio ??= new AudioContext();
    audio.resume();
    const start = audio.currentTime;
    [660, 880, 1320].forEach((frequency, i) => {
      const at = start + i * 0.09;
      const oscillator = audio.createOscillator();
      const gain = audio.createGain();
      oscillator.type = "square";
      oscillator.frequency.value = frequency;
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(0.08, at + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.08);
      oscillator.connect(gain).connect(audio.destination);
      oscillator.start(at);
      oscillator.stop(at + 0.09);
    });
  } catch {}
}

hydrateIcons();

$("new-game").addEventListener("click", () => openPage({ type: "add" }));
$("open-settings").addEventListener("click", () => openPage({ type: "settings" }));

document.querySelectorAll("#views button").forEach((node) => {
  node.addEventListener("click", () => {
    page = null;
    mainKey = "";
    call("set_view", { view: node.dataset.view });
  });
});

$("reload").addEventListener("click", () => state.active && call("reload_session", { sessionId: state.active }));
$("home").addEventListener("click", () => state.active && call("home_session", { sessionId: state.active }));
$("popout").addEventListener("click", () => state.active && call("pop_out", { sessionId: state.active }));
$("fill").addEventListener("click", () => state.active && call("vault_fill", { sessionId: state.active }));

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && page) closePage();
});

listen("state", (event) => {
  state = event.payload;
  render();
});

listen("overview", (event) => {
  overview = event.payload;
  renderSidebar();
  renderTiles();
  if (state.active) renderToolbar();
  if (mainKey === "panel") renderMain(true);
});

listen("alert", (event) => {
  const { title, body, sound } = event.payload;
  setStatus(`${title}: ${body}`, "alert");
  if (sound) beep();
});

call("get_state");
invoke("get_overview")
  .then((value) => {
    overview = value;
    render();
  })
  .catch(() => {});
