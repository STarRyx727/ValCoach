//! VALORANT map metadata, coordinate transforms, and area resolution.
//!
//! Uses Valorant-API map data (xMultiplier, yMultiplier, xScalarToAdd, yScalarToAdd)
//! to convert world coordinates to minimap coordinates, and callout regions to
//! resolve semantic area names.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use valcoach_domain::Vector3;

/// Map metadata from Valorant-API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapMeta {
    pub display_name: String,
    pub map_url: String,
    pub x_multiplier: f64,
    pub y_multiplier: f64,
    pub x_scalar_to_add: f64,
    pub y_scalar_to_add: f64,
    pub callouts: Vec<Callout>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Callout {
    pub region_name: String,
    pub super_region_name: String,
    pub location: CalloutLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalloutLocation {
    pub x: f64,
    pub y: f64,
}

/// Resolved map position with semantic area.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapPosition {
    pub world: Vector3,
    pub minimap_x: f64,
    pub minimap_y: f64,
    pub region: Option<String>,
    pub super_region: Option<String>,
}

/// A map resolver for a specific VALORANT map.
pub struct MapResolver {
    meta: MapMeta,
    callout_index: Vec<(CalloutLocation, String, String)>,
}

impl MapResolver {
    pub fn new(meta: MapMeta) -> Self {
        let callout_index = meta
            .callouts
            .iter()
            .filter(|c| !c.region_name.is_empty())
            .map(|c| (c.location.clone(), c.region_name.clone(), c.super_region_name.clone()))
            .collect();
        Self { meta, callout_index }
    }

    /// Convert world coordinates to minimap pixel coordinates.
    /// Valorant-API formula: map_x = world_y * xMultiplier + xScalarToAdd
    ///                        map_y = world_x * yMultiplier + yScalarToAdd
    /// Note: world X/Y are swapped relative to map axes.
    pub fn world_to_minimap(&self, pos: &Vector3) -> (f64, f64) {
        let map_x = pos.y * self.meta.x_multiplier + self.meta.x_scalar_to_add;
        let map_y = pos.x * self.meta.y_multiplier + self.meta.y_scalar_to_add;
        (map_x, map_y)
    }

    /// Resolve the nearest callout region for a world position.
    pub fn area_at(&self, pos: &Vector3) -> Option<&str> {
        if self.callout_index.is_empty() {
            return None;
        }
        let (mx, my) = self.world_to_minimap(pos);
        let mut best: Option<(&str, f64)> = None;
        for (loc, region, _super) in &self.callout_index {
            let dx = loc.x - mx;
            let dy = loc.y - my;
            let dist = dx * dx + dy * dy;
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((region.as_str(), dist));
            }
        }
        best.map(|(r, _)| r)
    }

    pub fn super_region_at(&self, pos: &Vector3) -> Option<&str> {
        if self.callout_index.is_empty() {
            return None;
        }
        let (mx, my) = self.world_to_minimap(pos);
        let mut best: Option<(&str, f64)> = None;
        for (loc, _region, super_region) in &self.callout_index {
            let dx = loc.x - mx;
            let dy = loc.y - my;
            let dist = dx * dx + dy * dy;
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((super_region.as_str(), dist));
            }
        }
        best.map(|(r, _)| r)
    }

    pub fn resolve(&self, pos: &Vector3) -> MapPosition {
        let (minimap_x, minimap_y) = self.world_to_minimap(pos);
        let region = self.area_at(pos).map(str::to_owned);
        let super_region = self.super_region_at(pos).map(str::to_owned);
        MapPosition {
            world: pos.clone(),
            minimap_x,
            minimap_y,
            region,
            super_region,
        }
    }

    pub fn display_name(&self) -> &str {
        &self.meta.display_name
    }
}

/// Registry of map resolvers keyed by map asset path.
pub struct MapRegistry {
    resolvers: HashMap<String, MapResolver>,
}

impl MapRegistry {
    pub fn new() -> Self {
        Self {
            resolvers: HashMap::new(),
        }
    }

    pub fn register(&mut self, map_url: &str, resolver: MapResolver) {
        self.resolvers.insert(map_url.to_owned(), resolver);
    }

    pub fn resolver_for(&self, map_asset_path: &str) -> Option<&MapResolver> {
        // Extract map name from asset path like "/Game/Maps/Bonsai/Bonsai"
        let map_name = map_asset_path
            .rsplit('/')
            .next()
            .unwrap_or("");
        self.resolvers.get(map_name)
    }

    pub fn resolve_area(&self, map_asset_path: &str, pos: &Vector3) -> Option<String> {
        self.resolver_for(map_asset_path)
            .and_then(|resolver| resolver.area_at(pos))
            .map(str::to_owned)
    }
}

impl Default for MapRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_meta() -> MapMeta {
        MapMeta {
            display_name: "Split".to_owned(),
            map_url: "Bonsai".to_owned(),
            x_multiplier: -0.145,
            y_multiplier: 0.145,
            x_scalar_to_add: 650.0,
            y_scalar_to_add: 350.0,
            callouts: vec![
                Callout {
                    region_name: "A Site".to_owned(),
                    super_region_name: "A".to_owned(),
                    location: CalloutLocation { x: 100.0, y: 200.0 },
                },
                Callout {
                    region_name: "A Main".to_owned(),
                    super_region_name: "A".to_owned(),
                    location: CalloutLocation { x: 50.0, y: 100.0 },
                },
                Callout {
                    region_name: "Mid".to_owned(),
                    super_region_name: "Mid".to_owned(),
                    location: CalloutLocation { x: 300.0, y: 300.0 },
                },
            ],
        }
    }

    #[test]
    fn world_to_minimap_swaps_axes() {
        let resolver = MapResolver::new(test_meta());
        let pos = Vector3 { x: 1000.0, y: 2000.0, z: 0.0 };
        let (mx, my) = resolver.world_to_minimap(&pos);
        // map_x = world_y * x_mult + x_scalar = 2000 * -0.145 + 650 = 360
        assert_eq!(mx, 360.0);
        // map_y = world_x * y_mult + y_scalar = 1000 * 0.145 + 350 = 495
        assert_eq!(my, 495.0);
    }

    #[test]
    fn nearest_callout_resolves() {
        let resolver = MapResolver::new(test_meta());
        let pos = Vector3 { x: 1000.0, y: 2000.0, z: 0.0 };
        // (360, 495) is closest to "A Site" at (100, 200) vs "Mid" at (300, 300)
        let area = resolver.area_at(&pos);
        assert!(area.is_some());
    }
}
