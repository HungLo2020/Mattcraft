package net.minecraft.world.entity.ai.village.poi;

import com.mojang.serialization.DynamicOps;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.Tag;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.chunk.storage.RegionFile;
import net.minecraft.world.level.chunk.storage.RegionFileVersion;
import net.minecraft.world.level.chunk.storage.RegionStorageInfo;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.DataOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Stream;

import it.unimi.dsi.fastutil.ints.Int2ObjectMap;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class NativePoiStorageTest {
	private static final int CURRENT_DATA_VERSION = 4295;
	private static final RegionStorageInfo STORAGE_INFO = new RegionStorageInfo("test", null, "poi-test");

	@BeforeAll
	static void bootstrapMinecraft() {
		SharedConstants.tryDetectVersion();
		Bootstrap.bootStrap();
	}

	@Test
	void generatedPoiChunksMatchJavaCodecForAllRegionCompressionTypes(@TempDir Path tempDir) throws IOException {
		List<RegionFileVersion> versions = List.of(
			RegionFileVersion.VERSION_GZIP,
			RegionFileVersion.VERSION_DEFLATE,
			RegionFileVersion.VERSION_NONE,
			RegionFileVersion.VERSION_LZ4
		);
		ChunkPos pos = new ChunkPos(0, 0);

		for (RegionFileVersion version : versions) {
			Path dir = tempDir.resolve("format-" + version.getId());
			Files.createDirectories(dir);
			Path regionPath = dir.resolve("r.0.0.mca");
			CompoundTag root = poiRoot(
				section(
					"4",
					true,
					record(10, 64, 11, "minecraft:armorer", 0),
					record(-2, 65, 17, "minecraft:meeting", 3)
				),
				section("-1", false)
			);
			writeJavaRegion(regionPath, dir, version, pos, root);

			List<SectionView> javaSections = javaSections(root);
			try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
				NativePoiStorage.DecodeResult rust = nativePoiStorage.decodeChunk(pos.x, pos.z);

				assertEquals(NativePoiStorage.OK, rust.result().status(), "format " + version.getId());
				assertTrue(rust.result().present(), "format " + version.getId());
				assertEquals(version.getId(), rust.result().compressionId(), "format " + version.getId());
				assertEquals(javaSections, sectionViews(rust.sections()), "format " + version.getId());
			}
		}
	}

	@Test
	void absentPoiChunkReportsNotPresent(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		writeJavaRegion(regionPath, tempDir, RegionFileVersion.VERSION_DEFLATE, pos, poiRoot(section("0", false)));

		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
			NativePoiStorage.DecodeResult rust = nativePoiStorage.decodeChunk(1, 0);

			assertEquals(NativePoiStorage.OK, rust.result().status());
			assertFalse(rust.result().present());
			assertEquals(List.of(), rust.sections());
		}
	}

	@Test
	void rustDecodesUnknownPoiTypeForJavaRegistryResolution(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag root = poiRoot(section("0", true, record(1, 2, 3, "minecraft:not_a_real_poi_type", 0)));
		writeJavaRegion(regionPath, tempDir, RegionFileVersion.VERSION_NONE, pos, root);

		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
			NativePoiStorage.DecodeResult rust = nativePoiStorage.decodeChunk(pos.x, pos.z);

			assertEquals(NativePoiStorage.OK, rust.result().status());
			assertEquals(ResourceLocation.withDefaultNamespace("not_a_real_poi_type"), rust.sections().getFirst().records().getFirst().type());
			assertTrue(BuiltInRegistries.POINT_OF_INTEREST_TYPE.getOptional(rust.sections().getFirst().records().getFirst().type()).isEmpty());
			assertThrows(
				NativePoiStorage.UnknownPoiTypeException.class,
				() -> NativePoiStorage.toPackedSections(rust, registryAccess(), LevelHeightAccessor.create(-64, 384))
			);
			assertThrows(IllegalStateException.class, () -> javaSections(root));
		}
	}

	@Test
	void rustPoiTapeConvertsToRegistryResolvedPackedSections(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag root = poiRoot(
			section(
				"4",
				true,
				record(10, 64, 11, "minecraft:armorer", 0),
				record(-2, 65, 17, "minecraft:meeting", 3)
			),
			section("-1", false)
		);
		writeJavaRegion(regionPath, tempDir, RegionFileVersion.VERSION_DEFLATE, pos, root);

		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
			NativePoiStorage.DecodeResult rust = nativePoiStorage.decodeChunk(pos.x, pos.z);
			Int2ObjectMap<PoiSection.Packed> sections = NativePoiStorage.toPackedSections(
				rust, registryAccess(), LevelHeightAccessor.create(-64, 384)
			);

			assertEquals(2, sections.size());
			assertTrue(sections.get(4).isValid());
			assertFalse(sections.get(-1).isValid());
			assertEquals(javaSections(root), packedSectionViews(sections));
		}
	}

	@Test
	void rustWritesCurrentVersionPoiTapeReadableByJavaCodec(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(2, 3);
		CompoundTag root = poiRoot(
			section(
				"0",
				true,
				record(32, 70, 48, "minecraft:home", 0),
				record(33, 70, 48, "minecraft:meeting", 1)
			),
			section("6", false, record(34, 101, 48, "minecraft:armorer", 2))
		);
		Int2ObjectMap<PoiSection.Packed> sections = packedSections(root);
		byte[] tape = NativePoiStorage.encodeTape(sections);

		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
			NativePoiStorage.WriteResult write = nativePoiStorage.writeChunk(pos.x, pos.z, RegionFileVersion.VERSION_DEFLATE.getId(), tape);

			assertEquals(NativePoiStorage.OK, write.status());
			assertTrue(write.present());
			assertEquals(RegionFileVersion.VERSION_DEFLATE.getId(), write.compressionId());
			assertEquals(2, write.sectionCount());
			assertEquals(3, write.recordCount());
			assertTrue(write.compressedLength() > 0);
			assertTrue(write.decompressedLength() > 0);
		}

		CompoundTag javaRoot = readJavaRegion(regionPath, pos);
		assertEquals(javaSections(root), javaSections(javaRoot));
		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
			NativePoiStorage.DecodeResult rust = nativePoiStorage.decodeChunk(pos.x, pos.z);

			assertEquals(javaSections(root), sectionViews(rust.sections()));
		}
	}

	@Test
	void rustWriteRejectsMalformedPoiTapeWithoutCreatingChunk(@TempDir Path tempDir) throws IOException {
		Path regionPath = tempDir.resolve("r.0.0.mca");
		ChunkPos pos = new ChunkPos(0, 0);

		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
			assertThrows(
				NativePoiStorage.WriteException.class,
				() -> nativePoiStorage.writeChunk(pos.x, pos.z, RegionFileVersion.VERSION_DEFLATE.getId(), new byte[]{1, 2, 3})
			);
			NativePoiStorage.DecodeResult rust = nativePoiStorage.decodeChunk(pos.x, pos.z);
			assertFalse(rust.result().present());
		}
	}

	@Test
	void malformedPoiSchemasFailSafely(@TempDir Path tempDir) throws IOException {
		Path oldVersion = tempDir.resolve("old-version.mca");
		ChunkPos pos = new ChunkPos(0, 0);
		CompoundTag oldRoot = poiRoot(section("0", true));
		oldRoot.putInt("DataVersion", CURRENT_DATA_VERSION - 1);
		writeJavaRegion(oldVersion, tempDir, RegionFileVersion.VERSION_NONE, pos, oldRoot);
		assertThrows(IOException.class, () -> decodeOnce(oldVersion, pos));

		Path missingRecords = tempDir.resolve("missing-records.mca");
		CompoundTag missingRoot = new CompoundTag();
		missingRoot.putInt("DataVersion", CURRENT_DATA_VERSION);
		CompoundTag sections = new CompoundTag();
		CompoundTag section = new CompoundTag();
		section.putBoolean("Valid", true);
		sections.put("0", section);
		missingRoot.put("Sections", sections);
		writeJavaRegion(missingRecords, tempDir, RegionFileVersion.VERSION_NONE, pos, missingRoot);
		assertThrows(IOException.class, () -> decodeOnce(missingRecords, pos));
	}

	@Test
	void copiedRealWorldPoiRegionMatchesJavaCodecWhenAvailable(@TempDir Path tempDir) throws IOException {
		Path poiDir = Path.of("run", "saves", "Origin", "poi");
		assumeTrue(Files.isDirectory(poiDir), "Origin POI directory is not available");
		RegionChunk sourceChunk = firstPresentOriginPoiChunk(poiDir);
		assumeTrue(sourceChunk != null, "Origin POI regions have no populated chunks");
		Path source = sourceChunk.regionPath();
		Path copy = tempDir.resolve("r.0.0.mca");
		Files.copy(source, copy);
		ChunkPos pos = new ChunkPos(sourceChunk.regionX() * 32 + sourceChunk.localX(), sourceChunk.regionZ() * 32 + sourceChunk.localZ());
		CompoundTag javaRoot = readJavaRegion(copy, pos);
		assumeTrue(javaRoot.getInt("DataVersion").orElse(0) == CURRENT_DATA_VERSION, "Origin POI chunk uses old schema and remains Java DFU-owned");

		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(copy)) {
			NativePoiStorage.DecodeResult rust = nativePoiStorage.decodeChunk(pos.x, pos.z);

			assertEquals(NativePoiStorage.OK, rust.result().status());
			assertTrue(rust.result().present());
			assertEquals(javaSections(javaRoot), sectionViews(rust.sections()));
		}
	}

	private static NativePoiStorage.DecodeResult decodeOnce(Path regionPath, ChunkPos pos) throws IOException {
		try (NativePoiStorage nativePoiStorage = NativePoiStorage.open(regionPath)) {
			return nativePoiStorage.decodeChunk(pos.x, pos.z);
		}
	}

	private static void writeJavaRegion(Path regionPath, Path dir, RegionFileVersion version, ChunkPos pos, CompoundTag tag) throws IOException {
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, dir, version, false);
			DataOutputStream output = regionFile.getChunkDataOutputStream(pos)) {
			NbtIo.write(tag, output);
		}
	}

	private static CompoundTag readJavaRegion(Path regionPath, ChunkPos pos) throws IOException {
		try (RegionFile regionFile = new RegionFile(STORAGE_INFO, regionPath, regionPath.getParent(), false);
			InputStream input = regionFile.getChunkDataInputStream(pos)) {
			assertNotNull(input);
			return NbtIo.read(new java.io.DataInputStream(input));
		}
	}

	private static CompoundTag poiRoot(NamedSection... namedSections) {
		CompoundTag root = new CompoundTag();
		root.putInt("DataVersion", CURRENT_DATA_VERSION);
		CompoundTag sections = new CompoundTag();
		for (NamedSection namedSection : namedSections) {
			sections.put(namedSection.name(), namedSection.tag());
		}
		root.put("Sections", sections);
		return root;
	}

	private static NamedSection section(String sectionY, boolean valid, CompoundTag... records) {
		CompoundTag tag = new CompoundTag();
		tag.putBoolean("Valid", valid);
		ListTag recordsTag = new ListTag();
		for (CompoundTag record : records) {
			recordsTag.add(record);
		}
		tag.put("Records", recordsTag);
		return new NamedSection(sectionY, tag);
	}

	private static CompoundTag record(int x, int y, int z, String type, int freeTickets) {
		CompoundTag tag = new CompoundTag();
		tag.putIntArray("pos", new int[]{x, y, z});
		tag.putString("type", type);
		tag.putInt("free_tickets", freeTickets);
		return tag;
	}

	private static List<SectionView> javaSections(CompoundTag root) {
		return packedSectionViews(packedSections(root));
	}

	private static Int2ObjectMap<PoiSection.Packed> packedSections(CompoundTag root) {
		CompoundTag sections = root.getCompoundOrEmpty("Sections");
		Int2ObjectMap<PoiSection.Packed> result = new it.unimi.dsi.fastutil.ints.Int2ObjectOpenHashMap<>();
		for (String key : sections.keySet()) {
			Tag tag = sections.get(key);
			PoiSection.Packed packed = PoiSection.Packed.CODEC.parse(registryOps(), tag).getOrThrow(message -> new IllegalStateException(message));
			result.put(Integer.parseInt(key), packed);
		}
		return result;
	}

	private static RecordView recordView(PoiRecord.Packed record) {
		return new RecordView(
			record.pos().getX(),
			record.pos().getY(),
			record.pos().getZ(),
			record.poiType()
				.unwrapKey()
				.map(key -> key.location())
				.orElseThrow(() -> new IllegalStateException("POI type has no registry key")),
			record.freeTickets()
		);
	}

	private static DynamicOps<Tag> registryOps() {
		return registryAccess().createSerializationContext(NbtOps.INSTANCE);
	}

	private static RegistryAccess registryAccess() {
		return RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
	}

	private static List<SectionView> sectionViews(List<NativePoiStorage.Section> sections) {
		return sections
			.stream()
			.map(section -> new SectionView(section.sectionY(), section.valid(), recordViews(section.records())))
			.sorted(Comparator.comparingInt(SectionView::sectionY))
			.toList();
	}

	private static List<RecordView> recordViews(List<NativePoiStorage.Record> records) {
		return records
			.stream()
			.map(record -> new RecordView(record.x(), record.y(), record.z(), record.type(), record.freeTickets()))
			.toList();
	}

	private static RegionChunk firstPresentOriginPoiChunk(Path poiDir) throws IOException {
		try (Stream<Path> paths = Files.list(poiDir)) {
			for (Path path : paths.filter(path -> path.getFileName().toString().endsWith(".mca")).sorted().toList()) {
				ChunkPos pos = firstPresentChunk(path);
				if (pos != null) {
					String[] parts = path.getFileName().toString().split("\\.");
					int regionX = Integer.parseInt(parts[1]);
					int regionZ = Integer.parseInt(parts[2]);
					return new RegionChunk(path, regionX, regionZ, pos.x, pos.z);
				}
			}
		}
		return null;
	}

	private static List<SectionView> packedSectionViews(Int2ObjectMap<PoiSection.Packed> sections) {
		return sections
			.int2ObjectEntrySet()
			.stream()
			.map(entry -> new SectionView(entry.getIntKey(), entry.getValue().isValid(), entry.getValue().records().stream().map(NativePoiStorageTest::recordView).toList()))
			.sorted(Comparator.comparingInt(SectionView::sectionY))
			.toList();
	}

	private static ChunkPos firstPresentChunk(Path regionPath) throws IOException {
		byte[] bytes = Files.readAllBytes(regionPath);
		for (int i = 0; i < 1024 && i * 4 + 3 < bytes.length; i++) {
			if (readInt(bytes, i * 4) != 0) {
				int localX = i & 31;
				int localZ = i >> 5;
				return new ChunkPos(localX, localZ);
			}
		}
		return null;
	}

	private static int readInt(byte[] bytes, int offset) {
		return (bytes[offset] & 0xFF) << 24
			| (bytes[offset + 1] & 0xFF) << 16
			| (bytes[offset + 2] & 0xFF) << 8
			| bytes[offset + 3] & 0xFF;
	}

	private record NamedSection(String name, CompoundTag tag) {
	}

	private record SectionView(int sectionY, boolean valid, List<RecordView> records) {
	}

	private record RecordView(int x, int y, int z, ResourceLocation type, int freeTickets) {
	}

	private record RegionChunk(Path regionPath, int regionX, int regionZ, int localX, int localZ) {
	}
}
