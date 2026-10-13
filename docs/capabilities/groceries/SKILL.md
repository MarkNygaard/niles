---
name: groceries
description: Groceries, grocery, shopping list, buy, milk, eggs, bread, coffee; nemlig basket, delivery, deliveries, offer, offers, indkøbslisten.
version: 1.0.0
---
# Groceries and nemlig

```yaml
groceries:
  add: "add <thing> to the list" | "we need <thing>" | "we're out of <thing>" | "put 2 <thing> on the shopping list"
  remove: "take <thing> off the list" | "we don't need <thing> after all"
  read: "what's on the shopping list"
  nemlig_basket: "what's in the nemlig basket"
  nemlig_delivery: "when are my groceries coming" | "when can nemlig deliver on Friday"
  nemlig_offers: "is coffee on offer"
  tools:
    add_to_grocery_list: items[{said, product?, quantity?}]; said = the person's own words
    remove_from_grocery_list: items in the person's own words
    read_grocery_list: what is still to be bought
    nemlig_basket: the nemlig.com basket, its cost and the reserved delivery time
    nemlig_delivery_times: delivery times in the coming week with prices and order-by deadline; date=YYYY-MM-DD for one day
    nemlig_next_delivery: when the next order arrives
    nemlig_offers: search offers by query, in Danish (kaffe, mælk, rugbrød)
  notes:
    - pass `product` only when the shop calls it something else; prefer one of the household's known products
    - confirm in the person's own words, not the product name; the voice cannot say every Danish name
    - adding something already on the list says so (already_on_list) rather than adding it twice
    - the list learns: an item checked off as a product makes the words it was asked for an alias of that product
    - the nemlig tools only read; delivery times are reserved, and the basket filled, in the app's Groceries page
    - paying and placing the order happen at nemlig.com, by a person
    - half an hour before a delivery window opens, the satellites say the groceries are coming
    - nemlig tools answer with a reason when no nemlig.com login is set up; say it plainly
  examples:
    - User: "we're out of milk and eggs" → add_to_grocery_list [{said: milk}, {said: eggs}]
    - User: "add two kilos of potatoes" → add_to_grocery_list [{said: potatoes, quantity: "2 kg"}]
    - User: "is butter on offer at nemlig?" → nemlig_offers query=smør
```
