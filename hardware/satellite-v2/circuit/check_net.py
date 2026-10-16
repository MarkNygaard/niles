"""Sanity checks on a SKiDL/KiCad netlist: part count, dangling nets, BOM fields."""
import sys
import sexpdata


def sym(x):
    return x.value() if isinstance(x, sexpdata.Symbol) else x


def find(lst, key):
    return [e for e in lst if isinstance(e, list) and e and sym(e[0]) == key]


def main(path):
    tree = sexpdata.loads(open(path, encoding="utf-8").read())
    comps = find(find(tree, "components")[0], "comp")
    nets = find(find(tree, "nets")[0], "net")
    print(f"{path}: {len(comps)} components, {len(nets)} nets")
    no_mpn = []
    for c in comps:
        ref = find(c, "ref")[0][1]
        fields = {sym(find(f, "name")[0][1]): (f[2] if len(f) > 2 else "") for f in find(find(c, "fields")[0], "field")} if find(c, "fields") else {}
        if not fields.get("MPN") and not fields.get("LCSC") and not ref.startswith(("TP", "FID", "H", "JP")):
            no_mpn.append(ref)
    single = []
    for n in nets:
        name = find(n, "name")[0][1]
        nodes = [(find(nd, "ref")[0][1], find(nd, "pin")[0][1]) for nd in find(n, "node")]
        if len(nodes) < 2:
            single.append((name, nodes))
    print("nets with one connection:", single or "none")
    print(f"parts without MPN/LCSC: {len(no_mpn)}", no_mpn[:40])


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "mainboard.net")
