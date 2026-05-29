//! niles-weather — Open-Meteo forecast + geocoding client.

pub mod client;
pub mod error;
pub mod model;
pub mod transport;

pub use client::{OpenMeteoClient, OpenMeteoConfig};
pub use error::{Error, Result};
pub use model::{
    CurrentConditions, DailyForecast, Location, Units, WeatherReport, weather_code_to_description,
};
pub use transport::{HttpTransport, WeatherTransport};
