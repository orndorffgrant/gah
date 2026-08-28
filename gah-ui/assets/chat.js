/* GAH chat: websocket streaming, simple message list. */
(() => {
    'use strict';

    const list = document.getElementById('messages');
    const form = document.getElementById('composer');
    const input = document.getElementById('prompt');
    const sendBtn = document.getElementById('send');
    const status = document.getElementById('status');
    const empty = document.getElementById('empty');
    const sessionPath = document.body.dataset.sessionPath;
    if (!list || !form || !input || !sessionPath) return;

    let streaming = false;

    /* ── transcript scrolling ────────────────────────────────────── */
    function pinned() {
        return list.scrollHeight - list.scrollTop - list.clientHeight < 90;
    }
    function scrollToBottom(force) {
        if (force || pinned()) list.scrollTop = list.scrollHeight;
    }

    /* ── composer auto-grow ──────────────────────────────────────── */
    function grow() {
        input.style.height = 'auto';
        input.style.height = Math.min(input.scrollHeight, 132) + 'px';
    }
    input.addEventListener('input', grow);

    /* ── rendering ───────────────────────────────────────────────── */
    function el(tag, cls, text) {
        const n = document.createElement(tag);
        if (cls) n.className = cls;
        if (text !== undefined) n.textContent = text;
        return n;
    }

    function addUser(text) {
        if (empty) empty.remove();
        const row = el('div', 'msg msg-user');
        row.appendChild(el('span', 'msg-role', 'You'));
        row.appendChild(el('div', 'msg-content', text));
        list.appendChild(row);
        scrollToBottom(true);
    }

    function newAssistant() {
        if (empty) empty.remove();
        const row = el('div', 'msg msg-assistant');
        row.appendChild(el('span', 'msg-role', 'Assistant'));
        const content = el('div', 'msg-content');
        const dots = el('div', 'dots');
        dots.appendChild(el('span'));
        dots.appendChild(el('span'));
        dots.appendChild(el('span'));
        content.appendChild(dots);
        row.appendChild(content);
        list.appendChild(row);
        scrollToBottom(true);
        return { content, dots, text: null };
    }

    function addError(text) {
        const row = el('div', 'msg msg-assistant');
        row.appendChild(el('span', 'msg-role', 'Error'));
        row.appendChild(el('div', 'msg-content chat-error', text));
        list.appendChild(row);
        scrollToBottom(true);
    }

    function prettyJson(s) {
        try { return JSON.stringify(JSON.parse(s), null, 2); } catch (_) { return s; }
    }

    function toolCard(icon, name, body, mod) {
        const card = el('div', 'tool-card' + (mod ? ' tool-card--' + mod : ''));
        const head = el('div', 'tool-head');
        head.appendChild(el('span', 'tool-ico' + (mod ? ' tool-ico--' + mod : ''), icon));
        head.appendChild(el('span', 'tool-name', name));
        card.appendChild(head);
        const pre = el('pre', 'tool-body', body.length > 800 ? body.slice(0, 800) + '\n\u2026' : body);
        card.appendChild(pre);
        return card;
    }

    function setStreaming(on) {
        streaming = on;
        sendBtn.disabled = on;
        status.dataset.state = on ? 'streaming' : 'idle';
    }

    /* ── websocket streaming ─────────────────────────────────────── */
    function send(prompt) {
        setStreaming(true);
        addUser(prompt);
        const state = newAssistant();

        let ws;
        try {
            const proto = location.protocol === 'https:' ? 'wss://' : 'ws://';
            ws = new WebSocket(proto + location.host + sessionPath + '/ws');
        } catch (_) {
            return fail('could not open connection');
        }

        let done = false;
        function fail(msg) {
            if (done) return;
            done = true;
            if (state.dots && state.dots.parentNode) state.dots.remove();
            addError(msg);
            setStreaming(false);
            try { ws.close(); } catch (_) {}
        }

        ws.onopen = () => ws.send(prompt);

        ws.onmessage = (ev) => {
            let frame;
            try { frame = JSON.parse(ev.data); } catch (_) { return; }
            switch (frame.type) {
                case 'text_delta': {
                    if (state.dots && state.dots.parentNode) state.dots.remove();
                    if (!state.text) {
                        state.text = el('div', 'stream-text');
                        state.content.appendChild(state.text);
                    }
                    state.text.textContent += frame.text;
                    scrollToBottom();
                    break;
                }
                case 'tool_call': {
                    if (state.dots && state.dots.parentNode) state.dots.remove();
                    state.content.appendChild(toolCard('\u25b8', frame.name, prettyJson(frame.arguments)));
                    scrollToBottom();
                    break;
                }
                case 'tool_result': {
                    const raw = String(frame.content == null ? '' : frame.content);
                    const body = prettyJson(raw);
                    state.content.appendChild(toolCard('\u2713', 'result', body, 'ok'));
                    scrollToBottom();
                    break;
                }
                case 'done': {
                    done = true;
                    if (state.dots && state.dots.parentNode) state.dots.remove();
                    if (state.text) {
                        state.text.textContent = frame.output;
                    } else {
                        state.content.appendChild(el('div', 'stream-text', frame.output));
                    }
                    if (frame.usage) {
                        state.content.appendChild(el('div', 'usage',
                            frame.usage.input_tokens + '\u2193 ' +
                            frame.usage.output_tokens + '\u2191 tokens'));
                    }
                    scrollToBottom(true);
                    setStreaming(false);
                    try { ws.close(); } catch (_) {}
                    break;
                }
                case 'error': {
                    fail(frame.message || 'agent error');
                    break;
                }
            }
        };

        ws.onerror = () => fail('connection error');
        ws.onclose = () => {
            if (!done) fail('connection lost');
            setStreaming(false);
        };
    }

    form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        if (streaming) return;
        const prompt = input.value.trim();
        if (!prompt) return;
        input.value = '';
        grow();
        send(prompt);
    });

    input.addEventListener('keydown', (ev) => {
        if (ev.key === 'Enter' && !ev.shiftKey) {
            ev.preventDefault();
            form.requestSubmit();
        }
    });
})();