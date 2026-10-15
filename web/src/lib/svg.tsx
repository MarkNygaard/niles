import { createElement } from "react";
import type { ReactElement, ReactNode } from "react";

/**
 * The elements and attributes a plugin's logo may use. The same list
 * Niles sanitised it against on install (`sanitise_logo` in
 * `crates/niles-plugins/src/check.rs`); kept here too so the app's
 * safety never rests on the server alone.
 */
const ELEMENTS = new Set([
  "svg",
  "g",
  "path",
  "circle",
  "ellipse",
  "rect",
  "line",
  "polyline",
  "polygon",
]);

const ATTRIBUTES: Record<string, string> = {
  viewBox: "viewBox",
  d: "d",
  cx: "cx",
  cy: "cy",
  r: "r",
  rx: "rx",
  ry: "ry",
  x: "x",
  y: "y",
  x1: "x1",
  y1: "y1",
  x2: "x2",
  y2: "y2",
  width: "width",
  height: "height",
  points: "points",
  fill: "fill",
  "fill-rule": "fillRule",
  "fill-opacity": "fillOpacity",
  "clip-rule": "clipRule",
  stroke: "stroke",
  "stroke-width": "strokeWidth",
  "stroke-linecap": "strokeLinecap",
  "stroke-linejoin": "strokeLinejoin",
  "stroke-miterlimit": "strokeMiterlimit",
  "stroke-dasharray": "strokeDasharray",
  "stroke-dashoffset": "strokeDashoffset",
  "stroke-opacity": "strokeOpacity",
  opacity: "opacity",
  transform: "transform",
};

/**
 * A plugin's logo as React elements, so it is drawn inline and its
 * `currentColor` takes the text colour around it, as the built-in
 * marks do; or `null` if it is not a logo this allows.
 *
 * Never `innerHTML`: the text is parsed as XML and rebuilt element by
 * element from the allowlist above, as React elements, so nothing in
 * it can be markup the page runs. An element outside the list, a
 * namespaced attribute, or a value pointing elsewhere (`url(`,
 * `javascript:`) refuses the whole logo rather than drawing part of it.
 */
export function svgToReact(svg: string, className?: string): ReactElement | null {
  let doc: Document;
  try {
    doc = new DOMParser().parseFromString(svg, "image/svg+xml");
  } catch {
    return null;
  }
  const root = doc.documentElement;
  if (!root || root.nodeName !== "svg" || doc.getElementsByTagName("parsererror").length > 0) {
    return null;
  }
  let key = 0;
  function build(element: Element, top: boolean): ReactElement | null {
    const name = element.localName;
    if (!ELEMENTS.has(name) || (element.namespaceURI && element.namespaceURI !== SVG_NS)) {
      return null;
    }
    const props: Record<string, unknown> = { key: key++ };
    for (const attribute of Array.from(element.attributes)) {
      if (top && attribute.name === "xmlns") continue;
      const prop = ATTRIBUTES[attribute.name];
      if (!prop || attribute.prefix) return null;
      const value = attribute.value.toLowerCase();
      if (value.includes("url(") || value.includes("javascript:")) return null;
      props[prop] = attribute.value;
    }
    const children: ReactNode[] = [];
    for (const child of Array.from(element.children)) {
      const built = build(child, false);
      if (!built) return null;
      children.push(built);
    }
    if (top) {
      // Sized by the page, like the built-in marks.
      delete props.width;
      delete props.height;
      props.className = className;
      props["aria-hidden"] = true;
      props.focusable = "false";
    }
    return createElement(name, props, ...children);
  }
  return build(root, true);
}

const SVG_NS = "http://www.w3.org/2000/svg";
