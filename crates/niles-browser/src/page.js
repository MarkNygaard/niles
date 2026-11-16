(() => {
  "use strict";
  // Niles's page script. It runs in an isolated world the page can't
  // see, reads the page as text with [n] references for the model, and
  // answers hit tests. It never reads a password's value. It also
  // fills a saved login or a code, only on the frame's own origin when
  // it is one of the login's, and never reads a filled value back.
  // It also reads what a checkout shows, for the order card, and
  // whether the page asks for card details, without reading a card
  // field's value.
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
    // A combobox that isn't an input is a custom drop-down: clicked,
    // and choose refuses it as not a <select>.
    if (role === "combobox") return "button";
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

  // Fields Niles filled, read as filled and never by their value.
  const filled = new WeakSet();
  // A WeakRef to the last field filled, for submit.
  let lastFilled = null;

  function deepAll(selector) {
    const out = [];
    const walk = (root) => {
      for (const el of root.querySelectorAll(selector)) {
        out.push(el);
        if (el.shadowRoot && el.shadowRoot.mode === "open") walk(el.shadowRoot);
      }
    };
    walk(document);
    return out;
  }

  function usable(el) {
    return el && el.isConnected && shown(el) && !el.disabled && !el.readOnly;
  }

  function on(origins) {
    return (
      location.protocol === "https:" &&
      Array.isArray(origins) &&
      origins.includes(location.origin)
    );
  }

  function passwordField() {
    const fields = deepAll("input[type=password]").filter(usable);
    const current = fields.filter((el) =>
      (el.getAttribute("autocomplete") || "")
        .split(/\s+/)
        .includes("current-password"),
    );
    if (current.length === 1) return current[0];
    const notNew = fields.filter(
      (el) => !(el.getAttribute("autocomplete") || "").includes("new-password"),
    );
    return notNew.length === 1 ? notNew[0] : null;
  }

  function usernameField(password) {
    const candidates = deepAll("input").filter((el) => {
      if (!usable(el)) return false;
      const type = (el.getAttribute("type") || "").toLowerCase();
      return ["", "text", "email", "tel"].includes(type);
    });
    if (password) {
      const scope = password.form
        ? candidates.filter((el) => el.form === password.form)
        : candidates;
      const before = scope.filter(
        (el) =>
          el.compareDocumentPosition(password) & Node.DOCUMENT_POSITION_FOLLOWING,
      );
      if (!before.length) return null;
      const marked = before.filter((el) => {
        const auto = (el.getAttribute("autocomplete") || "").toLowerCase();
        return auto.includes("username") || auto.includes("email");
      });
      return marked.length ? marked[marked.length - 1] : before[before.length - 1];
    }
    // A username-first page: no password field yet. Only a field marked
    // username, or the single candidate sharing a form with a password
    // input (hidden or not, which is how those forms serve password
    // managers), is taken — never a name or label guess, so a
    // newsletter email box isn't a username.
    const marked = candidates.filter((el) =>
      (el.getAttribute("autocomplete") || "").toLowerCase().includes("username"),
    );
    if (marked.length === 1) return marked[0];
    const withPassword = candidates.filter((el) => {
      if (!el.form) return false;
      return !!el.form.querySelector("input[type=password]");
    });
    return withPassword.length === 1 ? withPassword[0] : null;
  }

  const CODE_WORDS =
    /one[-_ ]?time|otp|2fa|two[-_ ]?factor|mfa|verif|security[-_ ]?code|auth(entication)?[-_ ]?code|passcode|totp|\bcode\b|\bpin\b/i;
  const NOT_CODE = /post|zip|promo|discount|voucher|coupon|gift|referral|rabat/i;

  function codeFields() {
    if (passwordField()) return [];
    const inputs = deepAll("input").filter((el) => {
      if (!usable(el)) return false;
      const type = (el.getAttribute("type") || "").toLowerCase();
      return ["", "text", "tel", "number"].includes(type);
    });
    const marked = inputs.filter((el) =>
      (el.getAttribute("autocomplete") || "")
        .toLowerCase()
        .includes("one-time-code"),
    );
    if (marked.length === 1 && marked[0].maxLength !== 1) return [marked[0]];
    const boxes = inputs.filter((el) => el.maxLength === 1);
    if (boxes.length >= 4 && boxes.length <= 8) {
      const groups = new Map();
      for (const el of boxes) {
        const key = el.form || el.parentElement;
        if (!groups.has(key)) groups.set(key, []);
        groups.get(key).push(el);
      }
      const runs = [...groups.values()].filter(
        (group) => group.length >= 4 && group.length <= 8,
      );
      if (runs.length === 1) {
        return runs[0].sort((a, b) =>
          a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING
            ? -1
            : 1,
        );
      }
    }
    const named = inputs.filter((el) => {
      const text = [
        el.getAttribute("name") || "",
        el.getAttribute("id") || "",
        el.getAttribute("aria-label") || "",
        el.getAttribute("placeholder") || "",
        label(el),
      ]
        .join(" ");
      return CODE_WORDS.test(text) && !NOT_CODE.test(text);
    });
    return named.length === 1 ? [named[0]] : [];
  }

  function captcha() {
    for (const el of deepAll("iframe")) {
      if (!shown(el)) continue;
      if (el.closest(".grecaptcha-badge")) continue;
      let url;
      try {
        url = new URL(el.src, location.href);
      } catch (_) {
        continue;
      }
      if (url.search.includes("size=invisible")) continue;
      const host = url.hostname;
      const onGoogle =
        /(^|\.)google\.com$/.test(host) || /(^|\.)recaptcha\.net$/.test(host);
      if (
        (onGoogle && url.pathname.includes("/recaptcha")) ||
        /(^|\.)hcaptcha\.com$/.test(host) ||
        host === "challenges.cloudflare.com" ||
        /(^|\.)arkoselabs\.com$/.test(host) ||
        /(^|\.)funcaptcha\.com$/.test(host) ||
        /(^|\.)captcha-delivery\.com$/.test(host) ||
        host.includes("captcha")
      ) {
        return true;
      }
    }
    for (const sel of [
      "#px-captcha",
      ".px-captcha",
      "#challenge-form",
      "#challenge-stage",
      '[id^="cf-chl"]',
    ]) {
      for (const el of deepAll(sel)) {
        if (shown(el)) return true;
      }
    }
    return false;
  }

  // Whole words in any script: JS's \b knows only ASCII letters, and
  // these words are Danish as well as English.
  const words = (alternatives) =>
    new RegExp("(?<![\\p{L}\\p{N}])(?:" + alternatives + ")(?![\\p{L}\\p{N}])", "iu");

  // Names and addresses split into words, so "cardNumber",
  // "card_number" and "/card-number.html" all read "card number".
  const spaced = (s) =>
    String(s || "")
      .replace(/([a-z])([A-Z])/g, "$1 $2")
      .replace(/[-_.:\/?=&#\[\]]+/g, " ");

  // An element's own text, without its children's.
  function ownText(el) {
    let s = "";
    for (const n of el.childNodes) {
      if (n.nodeType === Node.TEXT_NODE) s += " " + n.textContent;
    }
    return squeeze(s);
  }

  // The text a box shows outside `skip` and outside anything that acts,
  // such as a Change link, its pieces joined by commas.
  function shownText(box, skip) {
    const parts = [];
    const walk = (n) => {
      if (n === skip) return;
      if (n.nodeType === Node.TEXT_NODE) {
        const s = squeeze(n.textContent || "");
        if (s) parts.push(s);
        return;
      }
      if (n.nodeType !== Node.ELEMENT_NODE) return;
      if (SKIP.has(n.tagName) || n.matches(ACT) || !shown(n)) return;
      for (const child of n.childNodes) walk(child);
    };
    for (const child of box.childNodes) walk(child);
    return parts.join(", ");
  }

  const tokensOf = (el) => (el.getAttribute("autocomplete") || "").toLowerCase().split(/\s+/);

  const CARD_AUTOCOMPLETE = ["cc-number", "cc-csc", "cc-exp", "cc-exp-month", "cc-exp-year"];
  const CARD_FIELD = words(
    "card ?num(?:ber)?|card ?no|cardnumber|cc ?num(?:ber)?|ccnum|cc ?no|cvc2?|cvv2?|csc|kortnummer|kortnr|kontrolcifre",
  );
  const CARD_FRAME = words(
    "cards?|credit ?cards?|debit ?cards?|cardnumber|cvc|cvv|csc|secured ?fields|hosted ?fields?|kortnummer|betalingskort",
  );
  const NOT_CARD = words(
    "gift ?cards?|giftcards?|gift|vouchers?|coupons?|promo|loyalty|members?|membership|bonus|club|rewards?|library|gavekort|rabat|medlem",
  );
  const NOT_TYPED = ["hidden", "checkbox", "radio", "submit", "button", "image", "reset", "file"];

  // A page asks for card details with a field marked or named for
  // them, or with a payment provider's card frame, which names itself
  // in its title, its name or its address. Only what shows counts, and
  // no card field's value is read. A "security code" is left out: that
  // is a sign-in code's label too.
  function cardDetails() {
    for (const el of deepAll("input, select")) {
      if (!shown(el)) continue;
      if (NOT_TYPED.includes((el.getAttribute("type") || "").toLowerCase())) continue;
      if (tokensOf(el).some((token) => CARD_AUTOCOMPLETE.includes(token))) return true;
      const said = spaced(
        [
          el.getAttribute("name"),
          el.getAttribute("id"),
          el.getAttribute("aria-label"),
          el.getAttribute("placeholder"),
          label(el),
        ].join(" "),
      );
      if (CARD_FIELD.test(said) && !NOT_CARD.test(said)) return true;
    }
    for (const el of deepAll("iframe")) {
      if (!shown(el)) continue;
      // Only the frame's host and path: a query string can say "card"
      // about anything, and an empty src would resolve to the page's
      // own address.
      let where = "";
      const src = el.getAttribute("src");
      if (src) {
        try {
          const url = new URL(src, location.href);
          where = url.hostname + url.pathname;
        } catch (_) {}
      }
      const said = spaced(
        [el.getAttribute("title"), el.getAttribute("name"), el.getAttribute("aria-label"), where].join(" "),
      );
      if (CARD_FRAME.test(said) && !NOT_CARD.test(said)) return true;
    }
    return false;
  }

  const CHECKOUT_PATH = words(
    "checkout|checkouts|check out|kasse|kassen|kassa|payment|payments|betaling|pay|bestilling",
  );
  const CHECKOUT_TITLE = words(
    "checkout|review (?:your )?order|order review|place (?:your )?order|confirm (?:your )?order|payment|kasse|kassen|kassa|betaling|gennemfør (?:købet|ordren|bestillingen)",
  );

  // A checkout says so in its path, its title or its main heading.
  // Lower headings don't count, because a shop's footer heads its
  // payment methods on every page. A data: page's path is its whole
  // text, so only an https path counts.
  function atCheckout() {
    if (location.protocol === "https:" && CHECKOUT_PATH.test(spaced(location.pathname))) return true;
    if (CHECKOUT_TITLE.test(document.title || "")) return true;
    for (const el of deepAll("h1")) {
      if (!shown(el)) continue;
      const text = squeeze(el.innerText || "");
      if (text.length <= 80 && CHECKOUT_TITLE.test(text)) return true;
    }
    return false;
  }

  const TOTAL = words(
    "total|grand total|order total|total to pay|total due|amount due|amount to pay|to pay|i alt|at betale|samlet pris|samlet",
  );
  const BEST_TOTAL = words(
    "grand total|order total|total to pay|total due|amount due|amount to pay|to pay|at betale",
  );
  const NOT_TOTAL = words(
    "sub ?total|subtotal|delsum|mellemsum|savings|you save|saved|discounts?|rabat|besparelse",
  );
  const LEADS_NOT_TOTAL =
    /^(?:vat|tax|taxes|moms|heraf|shipping|delivery|fragt|levering|items?|varer|products?|produkter|basket|kurv)(?![\p{L}\p{N}])/iu;
  // An amount with its currency before or after it: "412,50 kr.",
  // "€41.25", "DKK 1.234,50", "412,-". Digits group by threes.
  const MONEY =
    /(?:[$€£¥]|(?<!\p{L})(?:kr|dkk|sek|nok|eur|usd|gbp|chf)\.?)\s?(?:\d{1,3}(?:[ .,']\d{3})+(?:[.,]\d{1,2})?|\d+(?:[.,]\d{1,2})?)|(?:\d{1,3}(?:[ .,']\d{3})+(?:[.,]\d{1,2})?|\d+(?:[.,]\d{1,2})?)\s?(?:[$€£¥]|(?:kr|dkk|sek|nok|eur|usd|gbp|chf)(?!\p{L})\.?|,-)/iu;
  // An amount alone, with its two decimals: "412,50".
  const AMOUNT = /(?:\d{1,3}(?:[ .,']\d{3})+|\d+)[.,]\d{2}(?!\d)/u;

  function amountIn(text) {
    const t = squeeze(text);
    const found = t.match(MONEY) || t.match(AMOUNT);
    return found ? cap(squeeze(found[0]), 40) : null;
  }

  // What a text says after its first total word, where the amount is.
  function afterTotal(text) {
    const t = squeeze(text);
    const found = t.match(TOTAL);
    return found ? t.slice(found.index + found[0].length) : t;
  }

  // The amount beside a total's label: in its own element, in its next
  // siblings, or in up to three ancestors small enough to be its row.
  function amountNear(el) {
    const own = squeeze(el.innerText || "");
    if (own.length <= 80) {
      const found = amountIn(afterTotal(own));
      if (found) return found;
    }
    let sibling = el.nextElementSibling;
    for (let i = 0; sibling && i < 3; i++, sibling = sibling.nextElementSibling) {
      const text = squeeze(sibling.innerText || "");
      if (text.length > 80) break;
      const found = amountIn(text);
      if (found) return found;
    }
    let box = el.parentElement;
    for (let i = 0; box && i < 3; i++, box = box.parentElement) {
      const text = squeeze(box.innerText || "");
      if (text.length > 200) break;
      const found = amountIn(afterTotal(text));
      if (found) return found;
    }
    return null;
  }

  // The order's total: an order total outranks a plain total, and of
  // two of a rank the later one wins, as a summary ends with its total.
  // A subtotal, a saving, a tax, shipping or an items total never counts.
  function total() {
    let best = null;
    let rank = 0;
    for (const el of deepAll("*")) {
      if (SKIP.has(el.tagName)) continue;
      const own = ownText(el);
      if (!own || own.length > 60) continue;
      if (!TOTAL.test(own) || NOT_TOTAL.test(own) || LEADS_NOT_TOTAL.test(own)) continue;
      if (!shown(el)) continue;
      const amount = amountNear(el);
      if (!amount) continue;
      const r = BEST_TOTAL.test(own) ? 2 : 1;
      if (r >= rank) {
        rank = r;
        best = amount;
      }
    }
    return best;
  }

  const DELIVERY_WORDS =
    "delivery address|deliver to|delivering to|delivery details|shipping address|ship to|shipping to|send to|leveringsadresse|levering til|leveres til|leveringsoplysninger|leveringssted";
  const LEADS_DELIVERY = new RegExp("^(?:" + DELIVERY_WORDS + ")(?![\\p{L}\\p{N}])", "iu");
  const INLINE_DELIVERY = new RegExp("^(?:" + DELIVERY_WORDS + ")\\s*:\\s*(.+)$", "iu");

  // A delivery address the page has in fields: those marked shipping,
  // else those not marked billing.
  function addressFields() {
    const fields = deepAll("input, textarea, select").filter(shown);
    const pick = (token) => {
      const marked = fields.filter((el) => tokensOf(el).includes(token));
      const el =
        marked.find((f) => tokensOf(f).includes("shipping")) ||
        marked.find((f) => !tokensOf(f).includes("billing"));
      if (!el) return "";
      if (el.tagName === "SELECT") {
        const chosen = el.options[el.selectedIndex];
        return chosen ? squeeze(chosen.text) : "";
      }
      return squeeze(el.value || "");
    };
    const street =
      pick("street-address") ||
      [pick("address-line1"), pick("address-line2")].filter(Boolean).join(", ");
    const town = [pick("postal-code"), pick("address-level2")].filter(Boolean).join(" ");
    return street && town ? cap(street + ", " + town, 200) : null;
  }

  // The delivery address a checkout shows: after "Delivery address:" on
  // its line, in the field such a label names, or under such a heading;
  // else the shipping address fields. A button or link that starts
  // "Ship to" is an action, not a label.
  function delivery() {
    for (const el of deepAll("*")) {
      if (SKIP.has(el.tagName) || el.matches(ACT)) continue;
      const own = ownText(el);
      if (!own || own.length > 300 || !LEADS_DELIVERY.test(own) || !shown(el)) continue;
      const inline = own.match(INLINE_DELIVERY);
      if (inline && squeeze(inline[1]).length >= 5) return cap(squeeze(inline[1]), 200);
      if (own.length > 60) continue;
      if (el.tagName === "LABEL" && el.control) {
        const value = squeeze(el.control.value || "");
        if (value.length >= 5) return cap(value, 200);
        continue;
      }
      let box = el.parentElement;
      for (let i = 0; box && i < 3; i++, box = box.parentElement) {
        const text = shownText(box, el);
        if (text.length > 300) break;
        if (text.length >= 5) return cap(text, 200);
      }
    }
    return addressFields();
  }

  // What a checkout shows, for the order card and the stop at card details.
  function checkout() {
    return {
      at_checkout: atCheckout(),
      total: total(),
      delivery: delivery(),
      card_details: cardDetails(),
    };
  }

  function set(el, value) {
    el.focus();
    el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    filled.add(el);
    lastFilled = new WeakRef(el);
  }

  function form(origins) {
    const seen = captcha();
    if (!on(origins)) {
      return { on: false, captcha: seen, password: false, username: false, code: false };
    }
    const password = passwordField();
    return {
      on: true,
      captcha: seen,
      password: !!password,
      username: password ? !!usernameField(password) : !!usernameField(null),
      code: codeFields().length > 0,
    };
  }

  function fill(origins, username, password) {
    if (!on(origins)) return { username: false, password: false };
    if (password != null) {
      const p = passwordField();
      if (!p) return { username: false, password: false };
      const u = username != null ? usernameField(p) : null;
      if (u) set(u, username);
      set(p, password);
      return { username: !!u, password: true };
    }
    if (username != null) {
      const u = usernameField(null);
      if (u) set(u, username);
      return { username: !!u, password: false };
    }
    return { username: false, password: false };
  }

  function submit(origins) {
    if (!on(origins)) return "no";
    const el = lastFilled && lastFilled.deref();
    if (!el || !el.isConnected) return "no";
    if (el.form) {
      for (const member of el.form.elements) {
        if (!usable(member)) continue;
        const type = (member.getAttribute("type") || "").toLowerCase();
        if (member.tagName === "INPUT" && ["submit", "image"].includes(type)) {
          member.click();
          return "clicked";
        }
        if (member.tagName === "BUTTON" && member.type === "submit") {
          member.click();
          return "clicked";
        }
      }
      try {
        el.form.requestSubmit();
        return "submitted";
      } catch (_) {
        return "no";
      }
    }
    el.focus();
    return "enter";
  }

  function code(origins, value) {
    if (!on(origins)) return false;
    const fields = codeFields();
    if (!fields.length) return false;
    if (fields.length === 1) {
      set(fields[0], value);
      return true;
    }
    if (value.length !== fields.length) return false;
    for (let i = 0; i < fields.length; i++) set(fields[i], value[i]);
    return true;
  }

  function storage(origins) {
    if (!on(origins)) return null;
    try {
      const items = [];
      for (let i = 0; i < localStorage.length; i++) {
        const key = localStorage.key(i);
        items.push([key, localStorage.getItem(key)]);
      }
      return { origin: location.origin, items };
    } catch (_) {
      return null;
    }
  }

  function restore(origins, items) {
    if (!on(origins)) return false;
    try {
      for (const [k, v] of items) localStorage.setItem(k, v);
      return true;
    } catch (_) {
      return false;
    }
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

    // Client coordinates: the view's top edge is 0, so an item whose
    // bottom is at or above it lies wholly out of sight above.
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
        if (filled.has(el)) {
          line += " (filled in by Niles)";
        } else {
          const value = squeeze(el.value || "");
          line += value ? ' = "' + cap(value, 200) + '"' : " (empty)";
        }
      } else if (k === "password") {
        line += filled.has(el) ? " (filled in by Niles)" : " (hidden)";
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
      prefix = "";
      current = line;
      flush();
    };

    const walkText = (node) => {
      const s = squeeze(node.textContent || "");
      if (!s) return;
      // A text node's rect marks where it sits on the page; wholly
      // above the view it is out of the read, and says there is more.
      const range = document.createRange();
      range.selectNodeContents(node);
      const rects = range.getClientRects();
      range.detach();
      if (rects.length && rects[0].bottom <= 0) {
        above = true;
        return;
      }
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
      // Wholly above the view an element is out of the read, and says
      // there is more above. Fixed elements stay: their rect is the
      // view's own.
      if (rectOf(el).bottom <= 0) {
        above = true;
        return;
      }
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
      captcha: captcha(),
      card_details: cardDetails(),
      asks_code: passwordField() === null && codeFields().length > 0,
      more_above: above || window.scrollY > 0,
      more_below:
        cut ||
        window.scrollY + window.innerHeight < document.documentElement.scrollHeight - 2,
    };
  }

  globalThis.__niles = {
    node,
    info,
    read,
    hit,
    form,
    captcha,
    checkout,
    fill,
    submit,
    code,
    storage,
    restore,
  };
})();
