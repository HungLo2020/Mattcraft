package net.vulkanic.world;

import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.SharedConstants;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.EntityType;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

class RustGalWorldEntityIdentityTest {
	@BeforeAll
	static void bootstrap() {
		SharedConstants.tryDetectVersion();
		Bootstrap.bootStrap();
	}

	@Test
	void resolvesCanonicalEntityTypeBeforeCopiedMeshExtraction() {
		EntityRenderState state = new EntityRenderState();
		assertNull(RustGalWorldPrimitiveRenderer.entityIdentity(state));

		state.entityType = EntityType.COW;
		assertEquals(ResourceLocation.withDefaultNamespace("cow"), RustGalWorldPrimitiveRenderer.entityIdentity(state));

		state.entityType = EntityType.CHICKEN;
		assertEquals(ResourceLocation.withDefaultNamespace("chicken"), RustGalWorldPrimitiveRenderer.entityIdentity(state));

		state.entityType = EntityType.RABBIT;
		assertEquals(ResourceLocation.withDefaultNamespace("rabbit"), RustGalWorldPrimitiveRenderer.entityIdentity(state));

		state.entityType = EntityType.WITHER_SKULL;
		assertEquals(ResourceLocation.withDefaultNamespace("wither_skull"), RustGalWorldPrimitiveRenderer.entityIdentity(state));
	}
}
