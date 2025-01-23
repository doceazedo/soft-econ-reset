use std::fs::{read_dir, File};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use flate2::Compression;
use mca::{RegionReader, RegionWriter};
use simdnbt::owned::{BaseNbt, NbtCompound, NbtList, NbtTag};
use rayon::prelude::*;
use simdnbt::borrow::{Nbt, NbtCompoundList};
use simdnbt::Error;

fn main() {
    let now = Instant::now();

    vec![
        Path::new("world/region"),
        Path::new("world_nether/DIM-1/region"),
        Path::new("world_the_end/DIM1/region"),
    ].iter().par_bridge().for_each(|path| {
        process_region_folder(path);
    });

    vec![
        Path::new("world/playerdata"),
        Path::new("world_nether/playerdata"),
        Path::new("world_the_end/playerdata"),
    ].iter().par_bridge().for_each(|path| {
        process_player_data_folder(path);
    });

    let elapsed = now.elapsed();
    println!("Elapsed: {:.2?}", elapsed);
}

fn process_player_data_folder(player_data_dir: &Path) {
    read_dir(player_data_dir)
        .expect("playerdata directory does not exist")
        .par_bridge()
        .for_each(|maybe_player_data_path| {
            if let Ok(player_data_path) = maybe_player_data_path {
                // println!("Processing player data file {:?}", player_data_path.path());
                if let Some(extension) = player_data_path.path().extension() {
                    if extension == "dat" {
                        process_player_data(player_data_path.path());
                    }
                }
            }
        });
}

fn process_player_data(player_data_path: PathBuf) {
    let mut player_data = Vec::new();
    File::open(&player_data_path).unwrap().read_to_end(&mut player_data).unwrap();
    let mut decoded_src_decoder = flate2::read::GzDecoder::new(&player_data[..]);
    let mut input = Vec::new();
    if Read::read_to_end(&mut decoded_src_decoder, &mut input).is_err() {
        input = player_data.to_vec();
    }
    let input = input.as_slice();

    match simdnbt::borrow::read(&mut Cursor::new(input)) {
        Ok(nbt) => {
            match nbt {
                Nbt::Some(base_nbt) => {
                    let mut tags = base_nbt
                        .as_compound()
                        .iter()
                        .map(|item| (item.0.to_owned(), item.1.to_owned()))
                        .collect::<Vec<_>>();

                    vec!["Inventory", "EnderItems"].iter().for_each(|inventory_name| {
                        let maybe_items = base_nbt
                            .list(inventory_name)
                            .and_then(|list| list.compounds());
                        if let Some(items) = maybe_items {
                            let updated_items = process_items_list(items);
                            tags = tags
                                .iter()
                                .map(|item| (item.0.to_owned(), item.1.to_owned()))
                                .filter(|tag| tag.0.to_string_lossy() != *inventory_name)
                                .collect::<Vec<_>>();
                            tags.push(((*inventory_name).into(), NbtTag::List(NbtList::Compound(updated_items))));
                        }
                    });

                    let updated_nbt = BaseNbt::new(
                        "Player",
                        NbtCompound::from_values(tags),
                    );

                    let mut updated_nbt_bytes = Vec::new();
                    updated_nbt.write(&mut updated_nbt_bytes);

                    let mut buf = vec![];
                    let mut encoder = flate2::write::GzEncoder::new(&mut buf, Compression::default());
                    encoder.write_all(updated_nbt_bytes.as_slice()).expect("Could not encode file");
                    encoder.finish().expect("Could not finish compression");

                    File::create(player_data_path).unwrap().write_all(&buf).expect("Could not write to file");
                }
                Nbt::None => {}
            }
        }
        Err(_) => {}
    }
}

fn process_region_folder(region_dir: &Path) {
    read_dir(region_dir)
        .expect("region directory does not exist")
        .par_bridge()
        .for_each(|maybe_region_file_path| {
            if let Ok(region_file_path) = maybe_region_file_path {
                println!("Processing region file {:?}", region_file_path.path());
                process_region_file(region_file_path.path());
            }
        });
}

fn process_region_file(region_file_path: PathBuf) {
    let mut data = Vec::new();
    File::open(&region_file_path).unwrap().read_to_end(&mut data).unwrap();

    let region = RegionReader::new(&data).unwrap();

    let mut writer = RegionWriter::new();
    let mut buf = vec![];

    get_chunk_positions().iter().for_each(|(x, z)| {
        let maybe_chunk = region.get_chunk(*x, *z).unwrap();
        if let Some(chunk) = maybe_chunk {
            let mut chunk_bytes = chunk.decompress().unwrap();
            let mut nbt = simdnbt::borrow::read(&mut Cursor::new(&*chunk_bytes)).unwrap().unwrap();
            let maybe_block_entities = nbt
                .list("block_entities")
                .and_then(|list| list.compounds());
            if let Some(mut block_entities) = maybe_block_entities {
                let updated_block_entities = block_entities.into_iter().map(|block_entity| {
                    let maybe_items = block_entity
                        .list("Items")
                        .and_then(|list| list.compounds());
                    if let Some(items) = maybe_items {
                        let updated_items = process_items_list(items);
                        let mut tags = block_entity
                            .iter()
                            .filter(|tag| tag.0.to_string_lossy() != "Items")
                            .map(|item| (item.0.to_owned(), item.1.to_owned()))
                            .collect::<Vec<_>>();
                        tags.push(("Items".into(), NbtTag::List(NbtList::Compound(updated_items))));
                        return NbtCompound::from_values(tags)
                    }

                    return block_entity.to_owned()
                })
                    .collect::<Vec<_>>();

                let mut tags = nbt.as_compound()
                    .iter()
                    .filter(|tag| tag.0.to_string_lossy() != "block_entities")
                    .map(|item| (item.0.to_owned(), item.1.to_owned()))
                    .collect::<Vec<_>>();
                tags.push(("block_entities".into(), NbtTag::List(NbtList::Compound(updated_block_entities))));
                let updated_nbt = BaseNbt::new(
                    "",
                    NbtCompound::from_values(tags),
                );

                let mut updated_chunk_bytes = Vec::new();
                updated_nbt.write(&mut updated_chunk_bytes);

                writer.push_chunk(&updated_chunk_bytes, (*x as u8, *z as u8)).expect("Could not push chunk data");
            } else {
                writer.push_chunk(&chunk_bytes, (*x as u8, *z as u8)).expect("Could not push chunk data");
            }
        }
    });

    writer.write(&mut buf).expect("Could not write to buffer");
    File::create(region_file_path).unwrap().write_all(&buf).expect("Could not write to file");
}

fn process_items_list(items: NbtCompoundList) -> Vec<NbtCompound> {
    items.clone().into_iter()
        // remove items
        .filter(|item|
            match item.string("id").unwrap().to_string_lossy().to_string().as_str() {
                "minecraft:totem_of_undying"
                | "minecraft:elytra" => false,

                _ => true,
            }
        )
        .map(|item| {
            let binding = item.clone().string("id").unwrap().to_string_lossy().to_string();
            let id = binding.as_str();

            match id {
                // reduce items
                "minecraft:end_crystal"
                | "minecraft:experience_bottle"
                | "minecraft:enchanted_golden_apple"
                | "minecraft:ender_chest"
                | "minecraft:tipped_arrow" => {
                    let maybe_count = item.int("count");
                    if let Some(count) = maybe_count {
                        println!("{}x {}", count, id);
                        let mut tags = item
                            .iter()
                            .filter(|tag| tag.0.to_string_lossy() != "count")
                            .map(|item| (item.0.to_owned(), item.1.to_owned()))
                            .collect::<Vec<(_)>>();
                        tags.push(("count".into(), NbtTag::Int(1)));
                        return NbtCompound::from_values(tags)
                    }
                    return item.to_owned()
                }

                // remove enchantments
                "minecraft:netherite_axe"
                | "minecraft:netherite_block"
                | "minecraft:netherite_boots"
                | "minecraft:netherite_chestplate"
                | "minecraft:netherite_helmet"
                | "minecraft:netherite_hoe"
                | "minecraft:netherite_ingot"
                | "minecraft:netherite_leggings"
                | "minecraft:netherite_pickaxe"
                | "minecraft:netherite_scrap"
                | "minecraft:netherite_shovel"
                | "minecraft:netherite_sword"
                | "minecraft:elytra" => {
                    let maybe_enchantments = item
                        .compound("components")
                        .and_then(|components| components.compound("minecraft:enchantments"))
                        .and_then(|enchantments| enchantments.compound("levels")); // probably safe to remove this
                    if let Some(enchantments) = maybe_enchantments {
                        println!("{:?}", enchantments);
                        let mut tags = item
                            .iter()
                            .map(|item| {
                                if item.0.to_string_lossy() == "components" {
                                    let maybe_components = item.1.compound();
                                    if let Some(components) = maybe_components {
                                        let filtered_components = components
                                            .iter()
                                            .filter(|tag| tag.0.to_string_lossy() != "minecraft:enchantments")
                                            .map(|item| (item.0.to_owned(), item.1.to_owned()))
                                            .collect::<Vec<_>>();
                                        return (item.0.to_owned(), NbtTag::Compound(NbtCompound::from_values(filtered_components)))
                                    }
                                    return (item.0.to_owned(), item.1.to_owned())
                                }
                                return (item.0.to_owned(), item.1.to_owned())
                            })
                            .collect::<Vec<_>>();
                        return NbtCompound::from_values(tags)
                    }

                    return item.to_owned()
                }

                "minecraft:shulker_box"
                | "minecraft:white_shulker_box"
                | "minecraft:orange_shulker_box"
                | "minecraft:magenta_shulker_box"
                | "minecraft:light_blue_shulker_box"
                | "minecraft:yellow_shulker_box"
                | "minecraft:lime_shulker_box"
                | "minecraft:pink_shulker_box"
                | "minecraft:gray_shulker_box"
                | "minecraft:silver_shulker_box"
                | "minecraft:cyan_shulker_box"
                | "minecraft:purple_shulker_box"
                | "minecraft:blue_shulker_box"
                | "minecraft:brown_shulker_box"
                | "minecraft:green_shulker_box"
                | "minecraft:red_shulker_box"
                | "minecraft:black_shulker_box" => {
                    let maybe_container = item
                        .compound("components")
                        .and_then(|components| components.list("minecraft:container"))
                        .and_then(|container| container.compounds());
                    if let Some(container) = maybe_container {
                        let updated_container = process_container_list(container);
                        let tags = item
                            .iter()
                            .map(|item| {
                                if item.0.to_string_lossy() == "components" {
                                    let maybe_components = item.1.compound();
                                    if let Some(components) = maybe_components {

                                        let mut filtered_components = components
                                            .iter()
                                            .filter(|tag| tag.0.to_string_lossy() != "minecraft:container")
                                            .map(|item| (item.0.to_owned(), item.1.to_owned()))
                                            .collect::<Vec<_>>();
                                        filtered_components.push(("minecraft:container".into(), NbtTag::List(NbtList::Compound(updated_container.clone()))));
                                        return (item.0.to_owned(), NbtTag::Compound(NbtCompound::from_values(filtered_components)))
                                    }
                                    return (item.0.to_owned(), item.1.to_owned())
                                }
                                return (item.0.to_owned(), item.1.to_owned())
                            })
                            .collect::<Vec<_>>();
                        return NbtCompound::from_values(tags)
                    }

                    return item.to_owned()
                }

                _ => item.to_owned()
            }
        })
        .collect::<Vec<_>>()
}

fn process_container_list(items: NbtCompoundList) -> Vec<NbtCompound> {
    let cloned_items = items.clone();
    cloned_items.into_iter()
        // remove items
        .filter(|item|
            match item.compound("item").unwrap().string("id").unwrap().to_string_lossy().to_string().as_str() {
                "minecraft:totem_of_undying"
                | "minecraft:elytra" => false,

                _ => true,
            }
        )
        .map(|item| {
            let binding = item.clone().compound("item").unwrap().string("id").unwrap().to_string_lossy().to_string();
            let id = binding.as_str();

            match id {
                // reduce items
                "minecraft:end_crystal"
                | "minecraft:experience_bottle"
                | "minecraft:enchanted_golden_apple"
                | "minecraft:ender_chest"
                | "minecraft:tipped_arrow" => {
                    let maybe_count = item.compound("item").unwrap().int("count");
                    if let Some(count) = maybe_count {
                        println!("{}x {}", count, id);
                        let mut tags = item
                            .compound("item")
                            .unwrap()
                            .iter()
                            .filter(|tag| tag.0.to_string_lossy() != "count")
                            .map(|item| (item.0.to_owned(), item.1.to_owned()))
                            .collect::<Vec<(_)>>();
                        tags.push(("count".into(), NbtTag::Int(1)));
                        let updated_item = NbtCompound::from_values(tags);

                        let container_item = vec![
                            ("item".into(), NbtTag::Compound(updated_item)),
                            ("slot".into(), NbtTag::Int(item.int("slot").unwrap())),
                        ];

                        return NbtCompound::from_values(container_item);
                    }
                    return item.to_owned()
                }

                // remove enchantments
                "minecraft:netherite_axe"
                | "minecraft:netherite_block"
                | "minecraft:netherite_boots"
                | "minecraft:netherite_chestplate"
                | "minecraft:netherite_helmet"
                | "minecraft:netherite_hoe"
                | "minecraft:netherite_ingot"
                | "minecraft:netherite_leggings"
                | "minecraft:netherite_pickaxe"
                | "minecraft:netherite_scrap"
                | "minecraft:netherite_shovel"
                | "minecraft:netherite_sword" => {
                    let maybe_enchantments = item
                        .compound("item")
                        .and_then(|item| item.compound("components"))
                        .and_then(|components| components.compound("minecraft:enchantments"))
                        .and_then(|enchantments| enchantments.compound("levels")); // probably safe to remove this
                    if let Some(enchantments) = maybe_enchantments {
                        println!("{:?}", enchantments);
                        let mut tags = item
                            .compound("item")
                            .unwrap()
                            .iter()
                            .map(|item| {
                                if item.0.to_string_lossy() == "components" {
                                    let maybe_components = item.1.compound();
                                    if let Some(components) = maybe_components {
                                        let filtered_components = components
                                            .iter()
                                            .filter(|tag| tag.0.to_string_lossy() != "minecraft:enchantments")
                                            .map(|item| (item.0.to_owned(), item.1.to_owned()))
                                            .collect::<Vec<_>>();
                                        return (item.0.to_owned(), NbtTag::Compound(NbtCompound::from_values(filtered_components)))
                                    }
                                    return (item.0.to_owned(), item.1.to_owned())
                                }
                                return (item.0.to_owned(), item.1.to_owned())
                            })
                            .collect::<Vec<_>>();
                        let updated_item = NbtCompound::from_values(tags);

                        let container_item = vec![
                            ("item".into(), NbtTag::Compound(updated_item)),
                            ("slot".into(), NbtTag::Int(item.int("slot").unwrap())),
                        ];

                        return NbtCompound::from_values(container_item);
                    }

                    return item.to_owned()
                }

                _ => item.to_owned()
            }
        })
        .collect::<Vec<_>>()
}

fn get_chunk_positions() -> Vec<(usize, usize)> {
    let mut positions = Vec::new();

    for x in 0..=31 {
        for z in 0..=31 {
            positions.push((x, z));
        }
    }

    positions
}