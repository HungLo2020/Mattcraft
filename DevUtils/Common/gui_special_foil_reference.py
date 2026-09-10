"""Fixed-coordinate GUI special-foil comparison; no candidate-fitted alignment."""
from pathlib import Path
from PIL import Image, ImageChops, ImageStat


def image_pair(frozen, current, scale):
    if frozen.size != current.size or scale not in (2, 3):
        raise ValueError("special foil requires equal extents and GUI scale 2 or 3")
    width, height = frozen.size
    from graphics_harness import flat_item_witness_layout
    witnesses, _ = flat_item_witness_layout(scale, frozen.size)
    rows = []
    for slot, witness in enumerate(witnesses):
        # The shared witness includes a one-pixel horizontal probe inset.
        # Whole-icon comparison uses the actual vanilla logical GUI placement.
        box = (witness[0]-1, witness[1], witness[2]-1, witness[3])
        x, y = box[:2]
        if x < 0 or y < 0 or box[2] > width or box[3] > height:
            raise ValueError("special foil fixture does not fit the captured viewport")
        images = [im.convert("RGB").crop(box) for im in (frozen, current)]
        error = max(ImageStat.Stat(ImageChops.difference(*images)).mean)
        visible = all(max(hi - lo for lo, hi in im.getextrema()) > 8 for im in images)
        rows.append(dict(slot=slot+1, box=box, max_mean_channel_error=error,
                         visible=visible, passed=visible and error <= 2.0))
    return dict(schema="gui-special-foil-icons-v1", scale=scale, probes=rows,
                passed=all(row["passed"] for row in rows))


def stopped_timing(receipt, scale, viewport):
    observed_timing(receipt, scale, viewport)
    return True


def observed_timing(receipt, scale, viewport, phase=None):
    from graphics_harness import flat_item_witness_layout
    if (not isinstance(receipt, dict) or receipt.get("enabled") is not True
            or receipt.get("complete") is not True or type(receipt.get("frameSequence")) is not int
            or receipt["frameSequence"] <= 0):
        raise ValueError("special foil requires capture-local timing")
    boxes, _ = flat_item_witness_layout(scale, viewport)
    expected = {((box[0]-1)//scale, box[1]//scale) for box in boxes[1:]}
    samples = receipt.get("samples")
    if not isinstance(samples, list) or not 1 <= len(samples) <= 8:
        raise ValueError("special foil requires observed foil draws")
    observed = set()
    ticks_by_position = {}
    for sample in samples:
        if not isinstance(sample, dict): raise ValueError("invalid special foil timing record")
        position = (sample.get("x"), sample.get("y"))
        if any(type(v) is not int for v in position) or position not in expected or position in observed:
            raise ValueError("ambiguous special foil timing slot")
        observed.add(position)
        if "scaledTicks" in sample:
            if type(sample["scaledTicks"]) is not int or not 0 <= sample["scaledTicks"] <= 2**63-1 or any(k in sample for k in ("clockMillis", "speed", "strength")):
                raise ValueError("special foil requires an actual reference clock")
            ticks = sample["scaledTicks"]
        elif (type(sample.get("clockMillis")) is not int or not 0 <= sample["clockMillis"] <= 2**63-1
              or type(sample.get("speed")) not in (int, float)
              or sample.get("speed") != (0 if phase is None else 0.5) or sample.get("strength") != 0.5):
            raise ValueError("special foil semantic clock/settings differ")
        else:
            ticks = min(2**63-1, int(float(sample["clockMillis"])*sample["speed"]*8))
        if phase is None:
            if ticks != 0: raise ValueError("stopped special foil clock is moving")
        elif (type(phase) is not int or not 0 <= phase < 330000 or ticks <= 0
              or (ticks-phase)%330000 > 512):
            raise ValueError("special foil missed requested natural animation phase")
        ticks_by_position[position] = ticks
    writes = {}
    write_records = receipt.get("atlasWrites", [])
    if not isinstance(write_records, list) or len(write_records) > 8:
        raise ValueError("invalid atlas write observations")
    for write in write_records:
        if not isinstance(write, dict): raise ValueError("invalid atlas write record")
        source = (write.get("x"), write.get("y"))
        cell = (write.get("atlasX"), write.get("atlasY"))
        if (any(type(v) is not int for v in source+cell) or source not in observed
                or any(v < 0 for v in cell) or cell in writes):
            raise ValueError("ambiguous capture-frame atlas write")
        writes[cell] = source
    direct = observed.copy()
    aliases = receipt.get("atlasAliases", [])
    if not isinstance(aliases, list) or len(aliases) > 8:
        raise ValueError("invalid atlas reuse observations")
    for alias in aliases:
        if not isinstance(alias, dict): raise ValueError("invalid atlas reuse record")
        target = (alias.get("x"), alias.get("y"))
        source = (alias.get("sourceX"), alias.get("sourceY"))
        cell = (alias.get("atlasX"), alias.get("atlasY"))
        if (any(type(v) is not int for v in target+source+cell) or target not in expected
                or target in observed or source not in direct or writes.get(cell) != source):
            raise ValueError("atlas reuse lacks its actual same-frame timed write")
        observed.add(target)
        ticks_by_position[target] = ticks_by_position[source]
    if observed != expected:
        raise ValueError("special foil requires all eight observed draws or proven same-frame atlas reuses")
    return [ticks_by_position[position] for position in sorted(expected)]


def matching_sources(frozen, current):
    # The unfoiled control alone cannot establish equivalent special items.
    # Require the actual selected clock/compass frames and their geometry/UVs.
    import re
    names = set(frozen)
    if (names != set(current) or len(names) != 3
            or "minecraft:item/apple" not in names
            or sum(bool(re.fullmatch(r"minecraft:item/clock_[0-9]{2}", n)) for n in names) != 1
            or sum(bool(re.fullmatch(r"minecraft:item/compass_[0-9]{2}", n)) for n in names) != 1
            or frozen != current):
        raise ValueError("special foil requires matching apple, clock and compass source inputs")
    return True


def temporal_images(before_frozen, before_current, after_frozen, after_current, scale):
    images = [im.convert("RGB") for im in (before_frozen, before_current, after_frozen, after_current)]
    if len({im.size for im in images}) != 1:
        raise ValueError("temporal special foil extents differ")
    before = image_pair(images[0], images[1], scale)
    after = image_pair(images[2], images[3], scale)
    if not before["passed"] or not after["passed"]:
        raise ValueError("both temporal special foil image pairs must pass independently")
    rows = []
    for probe in before["probes"]:
        pixels = [list(im.crop(probe["box"]).getdata()) for im in images]
        count = len(pixels[0])
        changes = [[sum(abs(pixels[b+2][i][c]-pixels[b][i][c]) for i in range(count))/count
                    for c in range(3)] for b in range(2)]
        delta_error = max(sum(abs((pixels[3][i][c]-pixels[1][i][c])
                                 -(pixels[2][i][c]-pixels[0][i][c])) for i in range(count))/count
                          for c in range(3))
        # Two independent <=2-level frame errors bound the signed delta by4.
        # The static control must stay static; every special icon must move on BOTH backends.
        changed = (all(max(values) <= 2 for values in changes) if probe["slot"] == 1
                   else all(max(values) > 4 for values in changes))
        rows.append(dict(slot=probe["slot"], changes=changes, delta_error=delta_error,
                         passed=changed and delta_error <= 4))
    return dict(passed=all(row["passed"] for row in rows), probes=rows)


def temporal_reference(reference, pair, scale, phase):
    import graphics_harness as h
    from contextlib import ExitStack
    if type(phase) is not int or phase == 10000:
        raise ValueError("temporal special foil needs a distinct second phase")
    prior = h.read_json(Path(reference)/h.MANIFEST_NAME)
    if not isinstance(prior, dict) or prior.get("success") is not True:
        raise ValueError("temporal special foil requires an accepted first-phase manifest")
    visual = prior.get("cross_repository_visual_parity", {})
    checked = report(visual, "special-foil", 10000)
    if not checked["passed"] or len(checked["pairs"]) != 1 or len(visual.get("pairs", [])) != 1:
        raise ValueError("first special foil phase no longer passes actual pixel/timing validation")
    before = visual["pairs"][0]
    for key in ("baseline_artifact", "current_artifact"):
        old = h.deterministic_capture_document(Path(before[key]))
        new = h.deterministic_capture_document(Path(pair[key]))
        if h.deterministic_visual_fixture_equivalence(old, new)["status"] != "passed":
            raise ValueError("special foil world/camera/settings changed between phases")
        if old.get("guiItemFoilSources") != new.get("guiItemFoilSources"):
            raise ValueError("special item source frames/geometry changed between phases")
        for document in (old, new):
            reload = document.get("worldResourceReload", {})
            if (reload.get("complete") is not True or reload.get("selectedAtCapture") !=
                    ["vanilla", "file/mattmc-special-item-foil-pattern"]):
                raise ValueError("temporal special foil requires the selected patterned pack after reload")
        metadata = [h.read_key_values(h.latest_capture_meta_path(Path(row[key]).parent/"capture"))
                    for row in (before, pair)]
        if any(meta.get("gui_resource_pack_scenario") != "special-item-foil-moving" for meta in metadata):
            raise ValueError("temporal special foil pack scenarios differ")
        digest_key = "gui_resource_pack_mattmc-special-item-foil-pattern_sha256"
        if key == "current_artifact" and (not metadata[0].get(digest_key)
                or metadata[0].get(digest_key) != metadata[1].get(digest_key)):
            raise ValueError("generated special foil pack bytes changed between phases")
    with ExitStack() as stack:
        images = [stack.enter_context(Image.open(row[key])) for row in (before, pair)
                  for key in ("baseline_image", "current_image")]
        return temporal_images(*images, scale)


def report(visual_report, fixture, phase=None, reference=None):
    if fixture != "special-foil":
        return dict(requested=False, passed=True, pairs=[])
    import graphics_harness as h
    from gui_foil_reference import source_evidence
    rows = []
    expected = dict(fixture="gui-special-foil-v1", complete=True, items=[
        dict(item="minecraft:"+("apple" if slot==0 else "clock" if slot%2 else "compass"),
             count=1, foil=slot>0) for slot in range(9)])
    for pair in visual_report.get("pairs", []):
        try:
            scales = []
            sources = []
            for key in ("baseline_artifact", "current_artifact"):
                artifact = Path(pair[key])
                doc = h.deterministic_capture_document(artifact) or {}
                if (doc.get("specialFoilFixture") != expected or doc.get("hotbarItemFixture") != fixture
                        or doc.get("guiItemFoilCount") != 8 or doc.get("guiItemGlintSpeed") != (0 if phase is None else 0.5)
                        or doc.get("guiItemGlintStrength") != 0.5 or doc.get("selectedHotbarSlot") != 1):
                    raise ValueError("special foil requires actual matching inventory/settings receipts")
                meta = h.latest_capture_meta_path(artifact.parent / "capture")
                if meta is None: raise ValueError("missing special foil capture metadata")
                scales.append(int(h.read_key_values(meta).get("forced_option_guiScale", "0")))
                sources.append(source_evidence(doc.get("guiItemFoilSources")))
            if scales[0] != scales[1]: raise ValueError("special foil GUI scales differ")
            matching_sources(*sources)
            with Image.open(pair["baseline_image"]) as frozen, Image.open(pair["current_image"]) as current:
                timings = [observed_timing(h.read_json(Path(str(pair[key])+".foil-timing.json")),
                           scales[0], frozen.size, phase) for key in ("baseline_image", "current_image")]
                row = image_pair(frozen, current, scales[0])
                row["observed_scaled_ticks"] = timings
                row["requested_phase"] = phase
                row["actual_fixture_sources_and_timing_verified"] = True
                if reference is not None:
                    row["temporal_change"] = temporal_reference(reference, pair, scales[0], phase)
                    row["passed"] &= row["temporal_change"]["passed"]
                rows.append(row)
        except (KeyError, OSError, TypeError, ValueError) as error:
            rows.append(dict(passed=False, reason=str(error)))
    return dict(requested=True, passed=bool(rows) and all(row["passed"] for row in rows), pairs=rows)
