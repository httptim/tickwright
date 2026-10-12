//! Item data the engine needs from the game: stack sizes and crafting
//! recipes, read from `data/<version>/`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

const MATERIALS_26_3: &str = include_str!("../../../data/26.3/materials.json");
const RECIPES_26_3: &str = include_str!("../../../data/26.3/recipes.json");

fn stack_sizes() -> &'static HashMap<String, i32> {
    static SIZES: OnceLock<HashMap<String, i32>> = OnceLock::new();
    SIZES.get_or_init(|| {
        let v: Value = serde_json::from_str(MATERIALS_26_3).expect("data/26.3/materials.json is valid");
        v["stack_sizes"]
            .as_object()
            .expect("stack_sizes")
            .iter()
            .map(|(k, n)| (k.clone(), n.as_i64().expect("a stack size") as i32))
            .collect()
    })
}

/// `Item.getDefaultMaxStackSize`.
pub fn max_stack_size(id: &str) -> i32 {
    let full;
    let id = if id.contains(':') {
        id
    } else {
        full = format!("minecraft:{id}");
        &full
    };
    stack_sizes().get(id).copied().unwrap_or(64)
}

/// `Item.getCraftingRemainder`: what a crafting slot leaves behind.
pub fn crafting_remainder(id: &str) -> Option<&'static str> {
    match id {
        "minecraft:milk_bucket"
        | "minecraft:water_bucket"
        | "minecraft:lava_bucket"
        | "minecraft:powder_snow_bucket" => Some("minecraft:bucket"),
        "minecraft:honey_bottle" | "minecraft:dragon_breath" => Some("minecraft:glass_bottle"),
        _ => None,
    }
}

/// An ingredient: the items that satisfy it.
#[derive(Debug, Clone)]
pub struct Ingredient(Vec<String>);

impl Ingredient {
    pub fn test(&self, id: &str) -> bool {
        self.0.iter().any(|i| i == id)
    }
}

#[derive(Debug, Clone)]
pub enum Recipe {
    Shaped { width: usize, height: usize, cells: Vec<Option<Ingredient>>, symmetrical: bool, result: (String, i32) },
    Shapeless { ingredients: Vec<Ingredient>, result: (String, i32) },
}

/// The shaped and shapeless crafting recipes, in the order the game's
/// recipe map tries them (by id).
pub fn recipes() -> &'static [Recipe] {
    static RECIPES: OnceLock<Vec<Recipe>> = OnceLock::new();
    RECIPES.get_or_init(|| parse_recipes(RECIPES_26_3))
}

fn parse_recipes(text: &str) -> Vec<Recipe> {
    let v: Value = serde_json::from_str(text).expect("data/26.3/recipes.json is valid");
    let tags = &v["tags"];
    let ingredient = |i: &Value| -> Ingredient {
        match i {
            Value::String(s) if s.starts_with('#') => Ingredient(
                tags[&s[1..]]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
            ),
            Value::String(s) => Ingredient(vec![s.clone()]),
            Value::Array(a) => Ingredient(a.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
            _ => Ingredient(Vec::new()),
        }
    };
    let result =
        |r: &Value| (r["result"]["id"].as_str().unwrap().to_string(), r["result"]["count"].as_i64().unwrap() as i32);
    v["recipes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| match r["type"].as_str().unwrap() {
            "shaped" => {
                let rows: Vec<&str> = r["pattern"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
                let width = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
                let cells = rows
                    .iter()
                    .flat_map(|row| {
                        let mut c: Vec<char> = row.chars().collect();
                        c.resize(width, ' ');
                        c
                    })
                    .map(|c| if c == ' ' { None } else { Some(ingredient(&r["key"][c.to_string()])) })
                    .collect();
                Recipe::Shaped {
                    width,
                    height: rows.len(),
                    cells,
                    symmetrical: r["symmetrical"].as_bool().unwrap_or(false),
                    result: result(r),
                }
            }
            _ => Recipe::Shapeless {
                ingredients: r["ingredients"].as_array().unwrap().iter().map(ingredient).collect(),
                result: result(r),
            },
        })
        .collect()
}

/// `CraftingInput`: a grid trimmed to the smallest box holding its items.
pub struct CraftingInput {
    pub width: usize,
    pub height: usize,
    pub items: Vec<Option<String>>,
}

impl CraftingInput {
    /// `CraftingInput.ofPositioned` for a `w` by `h` grid, row by row.
    pub fn of(w: usize, h: usize, grid: &[Option<String>]) -> CraftingInput {
        let filled = |x: usize, y: usize| grid[y * w + x].is_some();
        let cols: Vec<usize> = (0..w).filter(|&x| (0..h).any(|y| filled(x, y))).collect();
        let rows: Vec<usize> = (0..h).filter(|&y| (0..w).any(|x| filled(x, y))).collect();
        let (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) = (cols.first(), cols.last(), rows.first(), rows.last())
        else {
            return CraftingInput { width: 0, height: 0, items: Vec::new() };
        };
        let mut items = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                items.push(grid[y * w + x].clone());
            }
        }
        CraftingInput { width: x1 - x0 + 1, height: y1 - y0 + 1, items }
    }

    fn count(&self) -> usize {
        self.items.iter().filter(|i| i.is_some()).count()
    }
}

impl Recipe {
    pub fn result(&self) -> (&str, i32) {
        match self {
            Recipe::Shaped { result, .. } | Recipe::Shapeless { result, .. } => (&result.0, result.1),
        }
    }

    pub fn matches(&self, input: &CraftingInput) -> bool {
        match self {
            Recipe::Shaped { width, height, cells, symmetrical, .. } => {
                if input.count() != cells.iter().filter(|c| c.is_some()).count()
                    || input.width != *width
                    || input.height != *height
                {
                    return false;
                }
                let fits = |mirrored: bool| {
                    (0..*height).all(|y| {
                        (0..*width).all(|x| {
                            let px = if mirrored { width - 1 - x } else { x };
                            let item = &input.items[y * width + x];
                            match (&cells[y * width + px], item) {
                                (None, None) => true,
                                (Some(ing), Some(id)) => ing.test(id),
                                _ => false,
                            }
                        })
                    })
                };
                fits(false) || (!symmetrical && fits(true))
            }
            Recipe::Shapeless { ingredients, .. } => {
                let items: Vec<&String> = input.items.iter().flatten().collect();
                if items.len() != ingredients.len() {
                    return false;
                }
                // Match items to ingredients one to one.
                fn assign(items: &[&String], ingredients: &[Ingredient], used: &mut Vec<bool>) -> bool {
                    let Some((first, rest)) = items.split_first() else { return true };
                    for (i, ing) in ingredients.iter().enumerate() {
                        if !used[i] && ing.test(first) {
                            used[i] = true;
                            if assign(rest, ingredients, used) {
                                return true;
                            }
                            used[i] = false;
                        }
                    }
                    false
                }
                assign(&items, ingredients, &mut vec![false; ingredients.len()])
            }
        }
    }
}

/// `RecipeManager.getRecipeFor(CRAFTING, input)`: the first recipe that matches.
pub fn find_recipe(input: &CraftingInput) -> Option<&'static Recipe> {
    if input.items.is_empty() {
        return None;
    }
    recipes().iter().find(|r| r.matches(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cells: [&str; 9]) -> CraftingInput {
        let g: Vec<Option<String>> = cells.iter().map(|c| (!c.is_empty()).then(|| format!("minecraft:{c}"))).collect();
        CraftingInput::of(3, 3, &g)
    }

    #[test]
    fn stack_sizes_come_from_the_game() {
        assert_eq!(max_stack_size("minecraft:stone"), 64);
        assert_eq!(max_stack_size("minecraft:egg"), 16);
        assert_eq!(max_stack_size("cake"), 1);
    }

    #[test]
    fn recipes_match_shaped_shapeless_and_mirrored() {
        let iron = grid(["iron_ingot"; 9]);
        assert_eq!(find_recipe(&iron).unwrap().result(), ("minecraft:iron_block", 1));
        let table = grid(["", "", "", "oak_planks", "oak_planks", "", "oak_planks", "oak_planks", ""]);
        assert_eq!(find_recipe(&table).unwrap().result(), ("minecraft:crafting_table", 1));
        assert!(find_recipe(&grid(["oak_planks"; 9])).is_none());
        let cake =
            grid(["milk_bucket", "milk_bucket", "milk_bucket", "sugar", "egg", "sugar", "wheat", "wheat", "wheat"]);
        assert_eq!(find_recipe(&cake).unwrap().result(), ("minecraft:cake", 1));
        let planks = grid(["", "", "", "", "oak_log", "", "", "", ""]);
        assert_eq!(find_recipe(&planks).unwrap().result(), ("minecraft:oak_planks", 4));
    }
}
