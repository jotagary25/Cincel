//! Sample file rendered by the `phantom` example. It is not compiled: the
//! example embeds it with `include_str!` and `autoexamples` is disabled.

use std::collections::HashMap;
use std::fmt;

/// A line of an invoice.
#[derive(Clone, Debug)]
pub struct Item {
    pub sku: String,
    pub description: String,
    pub quantity: u32,
    pub unit_price: f64,
    pub tax_rate: f64,
}

impl Item {
    /// Total of the line before taxes.
    pub fn subtotal(&self) -> f64 {
        let quantity = self.quantity as f64;
        let gross = quantity * self.unit_price;
        let discount = if quantity >= 10.0 { 0.05 } else { 0.0 };
        gross * (1.0 - discount)
    }

    /// Taxes owed for this line.
    pub fn tax(&self) -> f64 {
        self.subtotal() * self.tax_rate
    }

    /// Total of the line, taxes included.
    pub fn total(&self) -> f64 {
        self.subtotal() + self.tax()
    }
}

impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:<10} {:<28} {:>4} x {:>8.2} = {:>10.2}",
            self.sku,
            self.description,
            self.quantity,
            self.unit_price,
            self.total()
        )
    }
}

/// An invoice: a customer, a currency and a list of items.
#[derive(Clone, Debug, Default)]
pub struct Invoice {
    pub number: u64,
    pub customer: String,
    pub currency: String,
    pub items: Vec<Item>,
}

impl Invoice {
    /// Creates an empty invoice.
    pub fn new(number: u64, customer: impl Into<String>) -> Self {
        Self {
            number,
            customer: customer.into(),
            currency: "EUR".to_string(),
            items: Vec::new(),
        }
    }

    /// Adds an item and returns the invoice, for chaining.
    pub fn with_item(mut self, item: Item) -> Self {
        self.items.push(item);
        self
    }

    /// Sum of every line before taxes.
    pub fn subtotal(&self) -> f64 {
        self.items.iter().map(Item::subtotal).sum()
    }

    /// Sum of the taxes of every line.
    pub fn taxes(&self) -> f64 {
        self.items.iter().map(Item::tax).sum()
    }

    /// Grand total.
    pub fn total(&self) -> f64 {
        self.subtotal() + self.taxes()
    }

    /// Taxes grouped by rate, so the report can print one row per rate.
    pub fn taxes_by_rate(&self) -> HashMap<String, f64> {
        let mut by_rate: HashMap<String, f64> = HashMap::new();
        for item in &self.items {
            let key = format!("{:.0}%", item.tax_rate * 100.0);
            *by_rate.entry(key).or_insert(0.0) += item.tax();
        }
        by_rate
    }

    /// Items whose total is above `threshold`.
    pub fn expensive_items(&self, threshold: f64) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| item.total() > threshold)
            .collect()
    }

    /// True when the invoice has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of units across every line.
    pub fn unit_count(&self) -> u32 {
        self.items.iter().map(|item| item.quantity).sum()
    }
}

/// Errors returned while validating an invoice.
#[derive(Debug, PartialEq, Eq)]
pub enum InvoiceError {
    NoItems,
    EmptyCustomer,
    UnknownCurrency(String),
    ZeroQuantity(String),
}

impl fmt::Display for InvoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvoiceError::NoItems => write!(f, "the invoice has no items"),
            InvoiceError::EmptyCustomer => write!(f, "the customer is empty"),
            InvoiceError::UnknownCurrency(code) => write!(f, "unknown currency: {code}"),
            InvoiceError::ZeroQuantity(sku) => write!(f, "zero quantity for {sku}"),
        }
    }
}

/// Currencies accepted by the billing service.
const CURRENCIES: [&str; 4] = ["EUR", "USD", "GBP", "CLP"];

/// Validates an invoice before sending it to the billing service.
pub fn validate(invoice: &Invoice) -> Result<(), InvoiceError> {
    if invoice.customer.trim().is_empty() {
        return Err(InvoiceError::EmptyCustomer);
    }
    if invoice.items.is_empty() {
        return Err(InvoiceError::NoItems);
    }
    if !CURRENCIES.contains(&invoice.currency.as_str()) {
        return Err(InvoiceError::UnknownCurrency(invoice.currency.clone()));
    }
    for item in &invoice.items {
        if item.quantity == 0 {
            return Err(InvoiceError::ZeroQuantity(item.sku.clone()));
        }
    }
    Ok(())
}

/// Renders the invoice as plain text.
pub fn render(invoice: &Invoice) -> String {
    let mut out = String::new();
    out.push_str(&format!("Invoice #{}\n", invoice.number));
    out.push_str(&format!("Customer: {}\n", invoice.customer));
    out.push_str(&"-".repeat(68));
    out.push('\n');
    for item in &invoice.items {
        out.push_str(&item.to_string());
        out.push('\n');
    }
    out.push_str(&"-".repeat(68));
    out.push('\n');
    out.push_str(&format!("Subtotal: {:>10.2}\n", invoice.subtotal()));
    out.push_str(&format!("Taxes:    {:>10.2}\n", invoice.taxes()));
    out.push_str(&format!("Total:    {:>10.2} {}\n", invoice.total(), invoice.currency));
    out
}

fn sample_invoice() -> Invoice {
    Invoice::new(1042, "Ferretería Bolívar")
        .with_item(Item {
            sku: "TRN-001".to_string(),
            description: "Taladro percutor".to_string(),
            quantity: 2,
            unit_price: 89.9,
            tax_rate: 0.21,
        })
        .with_item(Item {
            sku: "BRC-014".to_string(),
            description: "Broca widia 8 mm".to_string(),
            quantity: 24,
            unit_price: 1.35,
            tax_rate: 0.21,
        })
        .with_item(Item {
            sku: "GNT-007".to_string(),
            description: "Guantes de cuero".to_string(),
            quantity: 6,
            unit_price: 12.5,
            tax_rate: 0.10,
        })
}

fn main() {
    let invoice = sample_invoice();
    match validate(&invoice) {
        Ok(()) => print!("{}", render(&invoice)),
        Err(error) => eprintln!("invalid invoice: {error}"),
    }
    for (rate, amount) in invoice.taxes_by_rate() {
        println!("taxes at {rate}: {amount:.2}");
    }
    println!("units: {}", invoice.unit_count());
}
