(() => {
  "use strict";
  // Niles's page script. It runs in an isolated world the page can't
  // see, reads the page as text with [n] references for the model, and
  // answers hit tests. It never reads a password's value.
  if (globalThis.__niles) return;

  const refs = new Map(); // reference number -> WeakRef<Element>
  const ids = new WeakMap(); // Element -> reference number
  let next = 1;

  // A reference stays the same for an element across reads of one
  // document; a new document starts numbering anew.
  function ref(el) {
    let n = ids.get(el);
    if (!n) {
      n = next++;
      ids.set(el, n);
      refs.set(n, new WeakRef(el));
    }
    return n;
  }

  function node(n) {
    const el = refs.get(n) && refs.get(n).deref();
    return el && el.isConnected ? el : null;
  }

  const squeeze = (s) => String(s).replace(/\s+/g, " ").trim();
  const cap = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);

  function shown(el) {
    const rect = el.getBoundingClientRect();
    return (
      rect.width > 0 &&
      rect.height > 0 &&
      el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })
    );
  }

  const ACT =
    'a[href], button, input:not([type=hidden]), select, textarea, summary, [contenteditable=""], [contenteditable=true], [role=button], [role=link], [role=checkbox], [role=radio], [role=switch], [role=tab], [role=menuitem], [role=option], [role=combobox], [role=textbox], [role=searchbox]';

  function kind(el) {
    const role = el.getAttribute("role");
    const type = (el.getAttribute("type") || "").toLowerCase();
    if (el.tagName === "A" && el.href) return "link";
    if (role === "link") return "link";
    if (el.tagName === "BUTTON" || el.tagName === "SUMMARY") return "button";
    if (el.tagName === "INPUT" && ["submit", "button", "reset", "image"].includes(type))
      return "button";
    if (["button", "tab", "menuitem", "option"].includes(role)) return "button";
    if (el.tagName === "INPUT" && type === "checkbox") return "checkbox";
    if (["checkbox", "switch"].includes(role)) return "checkbox";
    if (el.tagName === "INPUT" && type === "radio") return "radio";
    if (role === "radio") return "radio";
    if (el.tagName === "INPUT" && type === "password") return "password";
    if (el.tagName === "INPUT" && type === "file") return "file";
    const textTypes = [
      "", "text", "search", "email", "tel", "url", "number",
      "date", "time", "datetime-local", "month", "week",
    ];
    if (el.tagName === "TEXTAREA") return "textbox";
    if (el.isContentEditable) return "textbox";
    if (["textbox", "searchbox"].includes(role)) return "textbox";
    if (el.tagName === "INPUT" && textTypes.includes(type)) {
      // A combobox role on an input is a text box with suggestions.
      return "textbox";
    }
    if (el.tagName === "SELECT") return "select";
    return "other";
  }

  function label(el) {
    const labelledBy = el.getAttribute("aria-labelledby");
    if (labelledBy) {
      const text = squeeze(
        labelledBy
          .split(/\s+/)
          .map((id) => document.getElementById(id))
          .filter(Boolean)
          .map((n) => n.innerText || n.textContent || "")
          .join(" ")
      );
      if (text) return cap(text, 160);
    }
    const aria = squeeze(el.getAttribute("aria-label") || "");
    if (aria) return cap(aria, 160);
    const isField =
      el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
    if (isField) {
      const labels = el.labels;
      if (labels && labels.length) {
        const text = squeeze(labels[0].innerText || labels[0].textContent || "");
        if (text) return cap(text, 160);
      }
      for (const attr of ["placeholder", "title", "name"]) {
        const text = squeeze(el.getAttribute(attr) || "");
        if (text) return cap(text, 160);
      }
      return "";
    }
    const inner = squeeze(el.innerText || "");
    if (inner) return cap(inner, 160);
    const title = squeeze(el.getAttribute("title") || "");
    if (title) return cap(title, 160);
    const img = el.querySelector("img[alt]");
    if (img) {
      const alt = squeeze(img.getAttribute("alt") || "");
      if (alt) return cap(alt, 160);
    }
    if (el.tagName === "INPUT") {
      const value = squeeze(el.getAttribute("value") || "");
      if (value) return cap(value, 160);
    }
    return "";
  }

  function options(el) {
    if (el.tagName !== "SELECT") return [];
    const out = [];
    for (const o of el.options) {
      if (out.length >= 200) break;
      out.push({ label: squeeze(o.label || o.text), value: o.value });
    }
    return out;
  }

  function info(el) {
    return {
      kind: kind(el),
      label: label(el),
      link: el.href != null ? el.href : null,
      in_form: !!(el.form || el.closest("form")) || el instanceof HTMLFormElement,
      disabled: !!(el.disabled || el.getAttribute("aria-disabled") === "true"),
      visible: shown(el),
      connected: el.isConnected,
      options: options(el),
    };
  }

  function hit(el, x, y) {
    const root = el.getRootNode();
    const doc = root.elementFromPoint ? root : document;
    const h = doc.elementFromPoint(x, y);
    return !!h && (h === el || el.contains(h));
  }

  const SKIP = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "SVG"]);
  const BREAK = new Set([
    "P", "DIV", "LI", "H1", "H2", "H3", "H4", "H5", "H6", "SECTION",
    "ARTICLE", "HEADER", "FOOTER", "NAV", "MAIN", "ASIDE", "TR", "TABLE",
    "UL", "OL", "FORM", "FIELDSET", "DT", "DD", "BLOCKQUOTE", "PRE",
    "FIGURE", "FIGCAPTION", "BR",
  ]);
  const HEADING = { H1: "# ", H2: "## ", H3: "### " };

  function read(budget) {
    const lines = [];
    let text = "";
    let cut = false;
    let above = false;
    let current = ""; // the line being built
    let prefix = ""; // heading or list marker for it

    const flush = () => {
      const line = squeeze(prefix + current);
      prefix = "";
      current = "";
      if (!line) return;
      if (lines.length && lines[lines.length - 1] === line) return; // drop dupes
      const capped = cap(line, 600);
      const joined = text ? text + "\n" + capped : capped;
      if (joined.length > budget) {
        cut = true;
        return;
      }
      text = joined;
      lines.push(line);
    };

    const view = {
      top: 0,
      bottom: window.innerHeight,
      // An item wholly above the view is out of sight; remember that
      // there is more above.
      note(rect) {
        if (rect.bottom <= this.top) above = true;
      },
    };

    const rectOf = (el) => el.getBoundingClientRect();

    const emitActionable = (el) => {
      const r = ref(el);
      const k = kind(el);
      const lab = label(el) || "this " + k;
      let line = "[" + r + "] " + k + ' "' + lab + '"';
      if (k === "link" && el.href) {
        let href = el.href;
        try {
          const target = new URL(href);
          if (target.origin === location.origin) href = target.pathname + target.search + target.hash;
        } catch (_) {}
        line += " → " + cap(href, 150);
      } else if (k === "textbox") {
        const value = squeeze(el.value || "");
        line += value ? ' = "' + cap(value, 200) + '"' : " (empty)";
      } else if (k === "password") {
        line += " (hidden)";
      } else if (k === "checkbox" || k === "radio") {
        line += el.checked ? " (checked)" : " (not checked)";
      } else if (k === "select") {
        const chosen = el.options[el.selectedIndex];
        const chosenText = chosen ? squeeze(chosen.label || chosen.text) : "";
        const shown = [];
        for (let i = 0; i < el.options.length && shown.length < 20; i++) {
          shown.push('"' + squeeze(el.options[i].label || el.options[i].text) + '"');
        }
        let opts = shown.join(", ");
        if (el.options.length > 20) opts += ", …and " + (el.options.length - 20) + " more";
        line += ' = "' + cap(chosenText, 200) + '" (options: ' + opts + ")";
      }
      if (el.disabled || el.getAttribute("aria-disabled") === "true") line += " (disabled)";
      view.note(rectOf(el));
      prefix = "";
      current = line;
      flush();
    };

    const walkText = (node) => {
      const s = squeeze(node.textContent || "");
      if (!s) return;
      // A text node's rect marks where it sits on the page.
      const range = document.createRange();
      range.selectNodeContents(node);
      const rects = range.getClientRects();
      range.detach();
      if (rects.length) view.note(rects[0]);
      current += (current ? " " : "") + s;
    };

    const walk = (n) => {
      if (cut) return;
      if (n.nodeType === Node.TEXT_NODE) {
        walkText(n);
        return;
      }
      if (n.nodeType !== Node.ELEMENT_NODE) return;
      const el = n;
      if (SKIP.has(el.tagName)) return;
      if (!shown(el)) return;
      if (el.tagName === "IFRAME") {
        const r = rectOf(el);
        if (r.width >= 50 && r.height >= 50) {
          let origin = "";
          try {
            origin = new URL(el.src).origin;
          } catch (_) {}
          current = "[a frame from " + (origin || "elsewhere") + " that Niles doesn't read]";
          flush();
        }
        return;
      }
      if (el.matches(ACT)) {
        emitActionable(el);
        return; // no further descent: its line carries it all
      }
      if (el.tagName === "BR") {
        flush();
        return;
      }
      const isTableCell = el.tagName === "TD" || el.tagName === "TH";
      if (BREAK.has(el.tagName) && !isTableCell) {
        flush();
        if (HEADING[el.tagName]) prefix = HEADING[el.tagName];
        if (el.tagName === "LI") prefix = "- ";
      }
      const inner =
        el.shadowRoot && el.shadowRoot.mode === "open"
          ? [el.shadowRoot]
          : el.tagName === "SLOT"
            ? el.assignedNodes({ flatten: true })
            : el.childNodes;
      for (const child of inner) walk(child);
      if (el.shadowRoot) return;
      if (isTableCell) {
        current += " | ";
      } else if (BREAK.has(el.tagName)) {
        flush();
      }
    };

    if (document.body) walk(document.body);
    flush();

    return {
      address: location.href,
      title: document.title,
      text,
      more_above: above || window.scrollY > 0,
      more_below:
        cut ||
        window.scrollY + window.innerHeight < document.documentElement.scrollHeight - 2,
    };
  }

  globalThis.__niles = { node, info, read, hit };
})();
