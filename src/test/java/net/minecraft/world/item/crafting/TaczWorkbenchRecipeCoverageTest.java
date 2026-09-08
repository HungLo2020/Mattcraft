package net.minecraft.world.item.crafting;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.Map;
import java.util.Set;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.TaczGunDefinitions;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.shapes.Shapes;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class TaczWorkbenchRecipeCoverageTest {
	@BeforeAll
	static void bootstrapMinecraft() {
		SharedConstants.tryDetectVersion();
		Bootstrap.bootStrap();
	}

	@Test
	void allRegisteredTaczGunsHaveWorkbenchRecipes() {
		assertRecipeCoverage(TaczWorkbenchRecipe.Category.GUN, TaczGunDefinitions.GUNS.size());
	}

	@Test
	void allRegisteredTaczAmmoHaveWorkbenchRecipes() {
		assertRecipeCoverage(TaczWorkbenchRecipe.Category.AMMO, TaczGunDefinitions.AMMO.size());
	}

	@Test
	void allRegisteredTaczAttachmentsHaveWorkbenchRecipes() {
		assertRecipeCoverage(TaczWorkbenchRecipe.Category.ATTACHMENT, TaczGunDefinitions.ATTACHMENTS.size());
	}

	@Test
	void allWorkbenchRecipesResolveToVisibleWorkbenchTabs() {
		Map<TaczWorkbenchRecipe.Category, Set<ResourceLocation>> allowedGroups = Map.of(
			TaczWorkbenchRecipe.Category.GUN,
			Set.of(
				group("pistol"), group("sniper"), group("rifle"), group("shotgun"), group("smg"), group("rpg"), group("mg"), group("misc")
			),
			TaczWorkbenchRecipe.Category.AMMO,
			Set.of(
				group("ammo"),
				group("pd_cartridges"),
				group("ifp_rifle_cartridges"),
				group("lc_specialized"),
				group("explosives"),
				group("shotgun_shells"),
				group("alternative_proj")
			),
			TaczWorkbenchRecipe.Category.ATTACHMENT,
			Set.of(group("scope"), group("muzzle"), group("stock"), group("grip"), group("extended_mag"), group("laser"))
		);
		for (TaczWorkbenchRecipe.Category category : TaczWorkbenchRecipe.Category.values()) {
			TaczWorkbenchRecipe.recipes(category).forEach(recipe -> assertTrue(
				allowedGroups.get(category).contains(recipe.group()),
				recipe.id() + " uses hidden workbench tab " + recipe.group()
			));
		}
	}

	@Test
	void workbenchModelsUseVanillaUvSpaceAndExistingTextures() {
		for (String workbench : Set.of("gun_smith_table", "ammo_workbench", "attachment_workbench")) {
			JsonObject model = loadJson("assets/minecraft/models/block/" + workbench + ".json");
			String texture = model.getAsJsonObject("textures").get("texture").getAsString();
			assertResourceExists("assets/minecraft/textures/" + texture.substring("minecraft:".length()).replace(':', '/') + ".png");
			for (JsonElement element : model.getAsJsonArray("elements")) {
				for (JsonElement coordinate : element.getAsJsonObject().getAsJsonArray("from")) {
					float value = coordinate.getAsFloat();
					assertTrue(value >= -16.0F && value <= 32.0F, workbench + " model has an out-of-range from coordinate: " + value);
				}
				for (JsonElement coordinate : element.getAsJsonObject().getAsJsonArray("to")) {
					float value = coordinate.getAsFloat();
					assertTrue(value >= -16.0F && value <= 32.0F, workbench + " model has an out-of-range to coordinate: " + value);
				}
				for (JsonElement face : element.getAsJsonObject().getAsJsonObject("faces").asMap().values()) {
					for (JsonElement uv : face.getAsJsonObject().getAsJsonArray("uv")) {
						float value = uv.getAsFloat();
						assertTrue(value >= -16.0F && value <= 16.0F, workbench + " model UV escaped vanilla model space: " + value);
					}
				}
			}
		}
		assertResourceExists("assets/minecraft/textures/gui/gun_smith_table.png");
		assertResourceExists("assets/minecraft/textures/gui/gun_smith_table_side.png");
	}

	@Test
	void workbenchBlocksDoNotOccludeNeighborFaces() {
		for (Block block : Set.of(Blocks.GUN_SMITH_TABLE, Blocks.AMMO_WORKBENCH, Blocks.ATTACHMENT_WORKBENCH)) {
			var state = block.defaultBlockState();
			assertFalse(state.canOcclude(), block + " should not cull neighboring block faces");
			assertFalse(state.useShapeForLightOcclusion(), block + " should not use its collision shape for occlusion");
			assertEquals(Shapes.empty(), state.getOcclusionShape(), block + " should expose no occlusion shape");
		}
	}

	private static void assertRecipeCoverage(TaczWorkbenchRecipe.Category category, int expectedCount) {
		assertEquals(expectedCount, TaczWorkbenchRecipe.recipes(category).size());
		TaczWorkbenchRecipe.recipes(category).forEach(recipe -> {
			assertFalse(recipe.ingredients().isEmpty(), recipe.id() + " should define materials");
			assertFalse(recipe.result().isEmpty(), recipe.id() + " should craft a registered item");
		});
	}

	private static ResourceLocation group(String group) {
		return ResourceLocation.withDefaultNamespace(group);
	}

	private static JsonObject loadJson(String path) {
		try (InputStream stream = TaczWorkbenchRecipeCoverageTest.class.getClassLoader().getResourceAsStream(path)) {
			assertTrue(stream != null, path + " should exist");
			return JsonParser.parseReader(new InputStreamReader(stream, StandardCharsets.UTF_8)).getAsJsonObject();
		} catch (Exception exception) {
			throw new AssertionError("Failed to load " + path, exception);
		}
	}

	private static void assertResourceExists(String path) {
		try (InputStream stream = TaczWorkbenchRecipeCoverageTest.class.getClassLoader().getResourceAsStream(path)) {
			assertTrue(stream != null, path + " should exist");
		} catch (Exception exception) {
			throw new AssertionError("Failed to check " + path, exception);
		}
	}
}
