//! Airless or near-airless rocky surfaces - the Moon and Mars - as a parameterised graph.
//!
//! The shipped templates are Earth landscapes: rivers, snow, sea. A Moon surface is shaped by
//! impacts and slow mass wasting instead, and Mars adds layered rock and wind-worn deposits on top.
//! The graph is the same skeleton for both, with the body choosing the weights.
//!
//! A real elevation model (a NASA DTM converted to 16-bit) can stand in for the procedural base
//! through a File node; the craters, wear and masks are then layered on top of it, gentler so the
//! measured relief survives.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::types::{Connection, Node, Position, SaveDefinition, Workflow};

/// Which body the surface imitates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Body {
    Moon,
    Mars,
}

impl Body {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "moon" | "lunar" => Some(Self::Moon),
            "mars" | "martian" => Some(Self::Mars),
            _ => None,
        }
    }
}

/// What the caller can steer.
pub struct PlanetParams {
    pub body: Body,
    pub seed: i64,
    /// Path to a 16-bit heightmap to build on instead of the procedural base.
    pub input_heightmap: Option<String>,
    /// 0..2, how many craters per area relative to the default.
    pub crater_density: f64,
    /// 0..2, how deep craters cut relative to the default.
    pub crater_depth: f64,
    /// Place one large hero crater.
    pub hero_crater: bool,
}

/// Node ids, fixed so callers and tests can address the outputs.
pub mod ids {
    pub const BASE: i32 = 10;
    pub const LARGE_CRATERS: i32 = 20;
    pub const SMALL_CRATERS: i32 = 30;
    pub const HERO_CRATER: i32 = 40;
    pub const POCKMARKS: i32 = 50;
    pub const STRATA: i32 = 55;
    pub const THERMAL: i32 = 60;
    pub const HEIGHT: i32 = 70;
    pub const SLOPE: i32 = 80;
    pub const RIMS: i32 = 90;
    pub const WEAR: i32 = 100;
    pub const DEPOSITS: i32 = 110;
}

/// The file names the build writes, by mask.
pub const OUTPUTS: [(&str, i32); 5] = [
    ("Height", ids::HEIGHT),
    ("Slope", ids::SLOPE),
    ("CraterRims", ids::RIMS),
    ("Wear", ids::WEAR),
    ("Deposits", ids::DEPOSITS),
];

fn node(id: i32, node_type: &str, name: &str, column: usize, properties: Value) -> Node {
    let properties: HashMap<String, Value> = properties
        .as_object()
        .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    Node {
        id,
        node_type: node_type.to_string(),
        name: name.to_string(),
        position: Position {
            x: 25000.0 + column as f64 * 350.0,
            y: 25000.0,
        },
        properties,
        ports: None,
        modifiers: None,
        save_definition: None,
    }
}

fn saved(mut node: Node, filename: &str) -> Node {
    node.save_definition = Some(SaveDefinition {
        filename: filename.to_string(),
        format: "PNG16".to_string(),
        enabled: true,
        disabled_profiles: Vec::new(),
    });
    node
}

fn link(from: i32, from_port: &str, to: i32) -> Connection {
    Connection {
        from_node: from,
        to_node: to,
        from_port: from_port.to_string(),
        to_port: "In".to_string(),
    }
}

/// Build the surface graph.
pub fn build_workflow(params: &PlanetParams) -> Workflow {
    let density = params.crater_density.clamp(0.0, 2.0);
    let depth = params.crater_depth.clamp(0.0, 2.0);
    // A measured surface already carries its own craters and relief; added impacts must stay
    // small beside it or they overwrite the data that was the reason to use it.
    let over_real = if params.input_heightmap.is_some() { 0.35 } else { 1.0 };
    let seed = params.seed;
    let mars = params.body == Body::Mars;

    let mut nodes = Vec::new();
    let mut connections = Vec::new();

    nodes.push(match &params.input_heightmap {
        Some(path) => node(ids::BASE, "File", "MeasuredSurface", 0, json!({ "FileName": path })),
        None => node(
            ids::BASE,
            "Perlin",
            "RollingBase",
            0,
            json!({
                "Scale": if mars { 0.35 } else { 0.25 },
                "Octaves": 10,
                "Gain": 0.45,
                "Seed": seed,
            }),
        ),
    });

    nodes.push(node(
        ids::LARGE_CRATERS,
        "CraterField",
        "LargeCraters",
        1,
        json!({
            "Scale": 3.0,
            "Depth": (1.2 * depth * over_real).max(0.0001),
            "Density": (0.6 * density).max(0.0001),
            "Seed": seed + 1,
        }),
    ));
    connections.push(link(ids::BASE, "Out", ids::LARGE_CRATERS));

    nodes.push(node(
        ids::SMALL_CRATERS,
        "CraterField",
        "SmallCraters",
        2,
        json!({
            "Scale": 0.8,
            "Depth": (0.8 * depth * over_real).max(0.0001),
            // the Moon keeps every small impact; Mars loses many to wind and old water
            "Density": ((if mars { 0.9 } else { 1.6 }) * density).clamp(0.0001, 2.0),
            "Seed": seed + 2,
        }),
    ));
    connections.push(link(ids::LARGE_CRATERS, "Out", ids::SMALL_CRATERS));

    let mut last = ids::SMALL_CRATERS;
    if params.hero_crater {
        nodes.push(node(
            ids::HERO_CRATER,
            "Crater",
            "HeroCrater",
            3,
            json!({
                "Scale": 0.35,
                "Height": (0.65 * depth * over_real).clamp(0.0001, 1.0),
                "Seed": seed + 3,
            }),
        ));
        connections.push(link(last, "Out", ids::HERO_CRATER));
        last = ids::HERO_CRATER;
    }

    nodes.push(node(
        ids::POCKMARKS,
        "Pockmarks",
        "Micrometeorites",
        4,
        json!({
            "Height": if mars { 0.3 } else { 0.6 },
            "Density": if mars { 0.35 } else { 0.5 },
            "Seed": seed + 4,
        }),
    ));
    connections.push(link(last, "Out", ids::POCKMARKS));
    last = ids::POCKMARKS;

    if mars {
        nodes.push(node(
            ids::STRATA,
            "Stratify",
            "LayeredRock",
            5,
            // Light layering: at 0.35 the strata covered every slope and read as a canyon wall.
            json!({ "Spacing": 0.15, "Intensity": 0.1, "Seed": seed + 5 }),
        ));
        connections.push(link(last, "Out", ids::STRATA));
        last = ids::STRATA;
    }

    nodes.push(node(
        ids::THERMAL,
        "Thermal2",
        "MassWasting",
        6,
        json!({
            "Duration": if mars { 0.08 } else { 0.04 },
            "Strength": if mars { 0.35 } else { 0.25 },
            "Angle": if mars { 30.0 } else { 34.0 },
        }),
    ));
    connections.push(link(last, "Out", ids::THERMAL));

    // A measured surface keeps its own range so the height in metres still holds; a procedural
    // one is stretched to use every step of the 16-bit export.
    let height = if params.input_heightmap.is_some() {
        node(ids::HEIGHT, "Clamp", "Height", 7, json!({}))
    } else {
        node(ids::HEIGHT, "Autolevel", "Height", 7, json!({}))
    };
    nodes.push(saved(height, "Height"));
    connections.push(link(ids::THERMAL, "Out", ids::HEIGHT));

    nodes.push(saved(node(ids::SLOPE, "Slope", "Slope", 8, json!({})), "Slope"));
    connections.push(link(ids::HEIGHT, "Out", ids::SLOPE));

    nodes.push(saved(
        node(ids::RIMS, "Curvature", "CraterRims", 8, json!({})),
        "CraterRims",
    ));
    connections.push(link(ids::HEIGHT, "Out", ids::RIMS));

    // Thermal2 writes Wear and Deposits in a sliver of the 16-bit range; levelling them makes
    // them usable as layer weights and PCG densities.
    nodes.push(saved(node(ids::WEAR, "Autolevel", "Wear", 8, json!({})), "Wear"));
    connections.push(link(ids::THERMAL, "Wear", ids::WEAR));

    nodes.push(saved(
        node(ids::DEPOSITS, "Autolevel", "Deposits", 8, json!({})),
        "Deposits",
    ));
    connections.push(link(ids::THERMAL, "Deposits", ids::DEPOSITS));

    Workflow { nodes, connections }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(body: Body, input: Option<&str>) -> PlanetParams {
        PlanetParams {
            body,
            seed: 7,
            input_heightmap: input.map(str::to_string),
            crater_density: 1.0,
            crater_depth: 1.0,
            hero_crater: true,
        }
    }

    #[test]
    fn every_connection_joins_existing_nodes() {
        for workflow in [
            build_workflow(&params(Body::Moon, None)),
            build_workflow(&params(Body::Mars, Some("C:/dtm.png"))),
        ] {
            let ids: Vec<i32> = workflow.nodes.iter().map(|n| n.id).collect();
            for c in &workflow.connections {
                assert!(ids.contains(&c.from_node), "missing source {}", c.from_node);
                assert!(ids.contains(&c.to_node), "missing target {}", c.to_node);
            }
        }
    }

    #[test]
    fn every_output_is_saved() {
        let workflow = build_workflow(&params(Body::Moon, None));
        for (name, id) in OUTPUTS {
            let node = workflow.nodes.iter().find(|n| n.id == id).unwrap();
            let save = node.save_definition.as_ref().expect(name);
            assert_eq!(save.filename, name);
            assert_eq!(save.format, "PNG16");
        }
    }

    #[test]
    fn a_measured_surface_replaces_the_procedural_base() {
        let workflow = build_workflow(&params(Body::Moon, Some("C:/lola.png")));
        let base = workflow.nodes.iter().find(|n| n.id == ids::BASE).unwrap();
        assert_eq!(base.node_type, "File");
        assert_eq!(base.properties["FileName"], json!("C:/lola.png"));
        // measured relief is kept, not stretched
        let height = workflow.nodes.iter().find(|n| n.id == ids::HEIGHT).unwrap();
        assert_ne!(height.node_type, "Autolevel");
    }

    #[test]
    fn only_mars_gets_layered_rock() {
        let has_strata = |w: &Workflow| w.nodes.iter().any(|n| n.id == ids::STRATA);
        assert!(!has_strata(&build_workflow(&params(Body::Moon, None))));
        assert!(has_strata(&build_workflow(&params(Body::Mars, None))));
    }
}
