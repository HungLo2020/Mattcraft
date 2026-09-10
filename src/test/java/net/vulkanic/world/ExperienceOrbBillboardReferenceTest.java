package net.vulkanic.world;

import net.blaze3d.vertex.PoseStack;
import org.joml.Quaternionf;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;

/** Executable oracle for the Rust placement test, using Frozen's PoseStack recipe.
 * Supplemental math coverage only; this does not assert live rendering parity. */
final class ExperienceOrbBillboardReferenceTest {

	@Test
	void frozenBillboardRecipeProducesNativeReferenceMatrix() {
		PoseStack pose = new PoseStack();
		pose.translate(1.25F, -0.7F, 2.5F);
		pose.mulPose(new Quaternionf().rotationY(0.25F));
		Quaternionf camera = new Quaternionf().rotationXYZ(0.37F, -0.81F, 0.12F);
		assertArrayEquals(new float[] {0.14553767F, -0.39673626F, -0.018175337F, 0.90613943F},
			new float[] {camera.x, camera.y, camera.z, camera.w}, 1.0e-7F);
		pose.translate(0.0F, 0.1F, 0.0F);
		pose.mulPose(camera);
		pose.scale(0.3F, 0.3F, 0.3F);
		assertArrayEquals(new float[] {
			0.2519499F, -0.044525683F, 0.15664832F, 0.0F,
			-0.0033460178F, 0.28709304F, 0.08698492F, 0.0F,
			-0.16281906F, -0.07479997F, 0.24061361F, 0.0F,
			1.25F, -0.59999996F, 2.5F, 1.0F
		}, pose.last().pose().get(new float[16]), 1.0e-6F);
	}

	@Test
	void verticalOffsetPrecedesCameraRotationAndUniformScale() {
		PoseStack pose = new PoseStack();
		pose.translate(0.0F, 0.1F, 0.0F);
		pose.mulPose(new Quaternionf().rotationZ((float) Math.PI / 2));
		pose.scale(0.3F, 0.3F, 0.3F);
		assertArrayEquals(new float[] {0.0F, 0.1F, 0.0F},
			new float[] {pose.last().pose().m30(), pose.last().pose().m31(), pose.last().pose().m32()},
			1.0e-7F);
	}
}
