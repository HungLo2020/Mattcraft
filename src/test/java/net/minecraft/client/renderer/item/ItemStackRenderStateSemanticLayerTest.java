package net.minecraft.client.renderer.item;

import java.util.List;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

class ItemStackRenderStateSemanticLayerTest {
	@org.junit.jupiter.api.BeforeAll
	static void bootstrap() {
		net.minecraft.SharedConstants.tryDetectVersion();
		net.minecraft.server.Bootstrap.bootStrap();
	}
	@Test
	void coverageReplayVisitsItemSemanticsWithoutEnteringNativeOwnership() {
		String property = "mattmc.dev.rustGalVulkanWholeFrame";
		String previous = System.getProperty(property);
		System.setProperty(property, "true");
		try {
			boolean[] coverage = {true};
			int[] visits = {0};
			var collector = (net.minecraft.client.renderer.SubmitNodeCollector) java.lang.reflect.Proxy.newProxyInstance(
				net.minecraft.client.renderer.SubmitNodeCollector.class.getClassLoader(),
				new Class<?>[] {net.minecraft.client.renderer.SubmitNodeCollector.class},
				(proxy, method, args) -> {
					if (method.getName().equals("isSemanticCoverageOnly")) return coverage[0];
					throw new AssertionError("unexpected callback: " + method.getName());
				});
			var renderer = new net.minecraft.client.renderer.special.SpecialModelRenderer<String>() {
				public void submit(String argument, net.minecraft.world.item.ItemDisplayContext context,
						net.blaze3d.vertex.PoseStack pose, net.minecraft.client.renderer.SubmitNodeCollector target,
						int light, int overlay, boolean foil, int outline) {
					visits[0]++;
					org.junit.jupiter.api.Assertions.assertEquals("coverage-payload", argument);
					org.junit.jupiter.api.Assertions.assertSame(collector, target);
					org.junit.jupiter.api.Assertions.assertEquals(net.minecraft.world.item.ItemDisplayContext.GROUND, context);
				}
				public void getExtents(java.util.Set<org.joml.Vector3f> set) {}
				public String extractArgument(net.minecraft.world.item.ItemStack stack) { return null; }
			};
			var state = new ItemStackRenderState();
			state.displayContext = net.minecraft.world.item.ItemDisplayContext.GROUND;
			state.newLayer().setupSpecialModel(renderer, "coverage-payload");
			var pose = new net.blaze3d.vertex.PoseStack();
			state.submitSemantic(pose, collector, 15728640, 0, 0);
			org.junit.jupiter.api.Assertions.assertEquals(1, visits[0]);
			org.junit.jupiter.api.Assertions.assertTrue(pose.isEmpty(), "coverage replay must restore its pose");
			coverage[0] = false;
			assertThrows(IllegalStateException.class,
				() -> state.submitSemantic(pose, collector, 15728640, 0, 0),
				"real submissions must still reject pre-selection handoff");
			org.junit.jupiter.api.Assertions.assertEquals(1, visits[0]);
		} finally {
			if (previous == null) System.clearProperty(property);
			else System.setProperty(property, previous);
		}
	}

	@Test
	void semanticLayerDefensivelyCopiesItsModelTransform() {
		float[] source = identityMatrix();
		ItemStackRenderState.SemanticLayer layer = new ItemStackRenderState.SemanticLayer(
			List.of(), new int[0], null, ItemStackRenderState.FoilType.NONE,
			false, false, true, source
		);

		source[0] = 7.0F;
		float[] extracted = layer.modelTransform();
		extracted[5] = 9.0F;

		assertArrayEquals(identityMatrix(), layer.modelTransform());
	}

	@Test
	void semanticLayerRejectsMalformedModelTransforms() {
		assertThrows(IllegalArgumentException.class, () -> new ItemStackRenderState.SemanticLayer(
			List.of(), new int[0], null, ItemStackRenderState.FoilType.NONE,
			false, false, true, new float[15]
		));
		float[] nonFinite = identityMatrix();
		nonFinite[3] = Float.NaN;
		assertThrows(IllegalArgumentException.class, () -> new ItemStackRenderState.SemanticLayer(
			List.of(), new int[0], null, ItemStackRenderState.FoilType.NONE,
			false, false, true, nonFinite
		));
	}

	private static float[] identityMatrix() {
		return new float[] {
			1.0F, 0.0F, 0.0F, 0.0F,
			0.0F, 1.0F, 0.0F, 0.0F,
			0.0F, 0.0F, 1.0F, 0.0F,
			0.0F, 0.0F, 0.0F, 1.0F
		};
	}
}
