use serde::{Deserialize, Serialize};

/// One thing nemlig.com sells, as much as choosing between them needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Product {
    /// nemlig's product number, as a string: it is one in their API.
    pub id: String,
    pub name: String,
    /// Size and brand, the way the site writes it: "1 l / Arla ØKO".
    pub description: String,
    /// Kroner.
    pub price: f64,
    /// "13,95 kr/l" — what tells two sizes apart.
    pub unit_price: Option<String>,
    pub image: Option<String>,
    /// In stock and deliverable to this account's address.
    pub available: bool,
}

/// A product as the search gateway sends it.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct Found {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    price: f64,
    unit_price_calc: Option<f64>,
    unit_price_label: Option<String>,
    primary_image: Option<String>,
    #[serde(default)]
    availability: Option<Availability>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Availability {
    #[serde(default)]
    is_delivery_available: bool,
    #[serde(default)]
    is_available_in_stock: bool,
}

impl From<Found> for Product {
    fn from(f: Found) -> Self {
        let unit_price = match (f.unit_price_calc, f.unit_price_label) {
            (Some(value), Some(label)) if !label.is_empty() => {
                // Their label is "kr/l"; the number is written the
                // Danish way, because the label is Danish.
                let (_, per) = label.split_once('/').unwrap_or(("kr", &label));
                Some(format!("{} kr/{per}", kroner(value)))
            }
            _ => None,
        };
        Self {
            id: f.id,
            name: f.name,
            description: f.description,
            price: f.price,
            unit_price,
            // The site writes a double slash after the host. It works,
            // but tidied it is one less thing that looks wrong in a log.
            image: f
                .primary_image
                .map(|url| url.replacen(".com//", ".com/", 1)),
            available: f
                .availability
                .is_none_or(|a| a.is_delivery_available && a.is_available_in_stock),
        }
    }
}

/// 13.95 → "13,95".
fn kroner(value: f64) -> String {
    format!("{value:.2}").replace('.', ",")
}

/// The product list inside a search answer.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct SearchAnswer {
    #[serde(default)]
    products: Option<ProductPage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ProductPage {
    #[serde(default)]
    products: Vec<Found>,
}

impl SearchAnswer {
    pub(crate) fn into_products(self) -> Vec<Product> {
        self.products
            .map(|page| page.products.into_iter().map(Product::from).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real answer for "letmælk".
    const ANSWER: &str = r#"{
        "Products": {"NumFound": 2, "Start": 0, "Products": [
            {"Id": "701012", "Name": "Letmælk 1,5% øko.", "Description": "1 l / Arla ØKO",
             "Price": 13.95, "UnitPriceCalc": 13.95, "UnitPriceLabel": "kr/l",
             "PrimaryImage": "https://nemlig.com//scommerce/images/letmaelk-1-5-oeko.jpg?i=yZOaOThd/701012",
             "Availability": {"IsDeliveryAvailable": true, "IsAvailableInStock": true, "ReasonMessageKeys": []},
             "Brand": "Arla", "Labels": ["Øko (dansk)"]},
            {"Id": "102650", "Name": "Letmælk øko.", "Description": "0,50 l / Naturmælk",
             "Price": 11.65, "UnitPriceCalc": 23.3, "UnitPriceLabel": "kr/l",
             "PrimaryImage": null,
             "Availability": {"IsDeliveryAvailable": true, "IsAvailableInStock": false}}
        ]},
        "Ads": [], "Recipes": []
    }"#;

    fn products() -> Vec<Product> {
        serde_json::from_str::<SearchAnswer>(ANSWER)
            .expect("parses")
            .into_products()
    }

    #[test]
    fn reads_what_choosing_needs() {
        let milk = &products()[0];
        assert_eq!(milk.id, "701012");
        assert_eq!(milk.name, "Letmælk 1,5% øko.");
        assert_eq!(milk.description, "1 l / Arla ØKO");
        assert_eq!(milk.price, 13.95);
        assert_eq!(milk.unit_price.as_deref(), Some("13,95 kr/l"));
        assert!(milk.available);
    }

    #[test]
    fn tidies_the_image_address() {
        assert_eq!(
            products()[0].image.as_deref(),
            Some("https://nemlig.com/scommerce/images/letmaelk-1-5-oeko.jpg?i=yZOaOThd/701012")
        );
        assert_eq!(products()[1].image, None);
    }

    #[test]
    fn out_of_stock_is_not_available() {
        assert!(!products()[1].available);
    }

    #[test]
    fn an_answer_with_no_products_is_an_empty_list() {
        let answer: SearchAnswer = serde_json::from_str(r#"{"Ads": []}"#).unwrap();
        assert!(answer.into_products().is_empty());
    }
}
