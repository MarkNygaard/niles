# Spot prices

Danish electricity spot prices in Niles, from
[Energi Data Service](https://www.energidataservice.dk/), Energinet's free open
data API. No key needed.

- **A device,** `system/electricity`: a sensor with the price now (`price`,
  kr/kWh) and the hour's place among today's hours (`hour_rank`, 1 the
  cheapest). Rules can follow either, for "when power drops below a krone,
  turn on the dishwasher plug".
- **A tool,** `spotprice_prices`, for "when is power cheapest tomorrow?".
- **A page,** Electricity in the menu: the price now, today's prices as a bar
  chart and a **Refresh now** button.

Settings: the price area (DK1 west of the Great Belt, DK2 east) and how many
hours a day the page calls cheap. These are spot prices only: network tariffs,
taxes and VAT come on top.

It is also the author guide's worked example, which walks through the code:
[Worked example: spot prices](https://getniles.app/docs/writing-add-ons/example-spotprice).

## Install

In Niles, on the Add-ons page's **Available** tab, press **Add** next to
**Add-on from GitHub** and paste `MarkNygaard/niles/spotprice`. Niles 0.5 and older don't
know addresses with an add-on's name after the repo.

## Build and check

From `addons/`, with the `wasm32-wasip2` target installed:

```sh
./build.sh spotprice
```

Then, from the repo's root:

```sh
cargo run -p niles-bin -- plugin check addons/spotprice
```

It builds against the frozen interface 0.2 (`schemas/niles-integration-0.2.wit`),
which Niles still runs.
