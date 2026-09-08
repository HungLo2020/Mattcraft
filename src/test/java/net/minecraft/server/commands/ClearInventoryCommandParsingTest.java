package net.minecraft.server.commands;

import com.mojang.brigadier.StringReader;
import com.mojang.brigadier.exceptions.CommandSyntaxException;
import net.minecraft.SharedConstants;
import net.minecraft.commands.CommandBuildContext;
import net.minecraft.commands.arguments.item.ItemPredicateArgument;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.flag.FeatureFlags;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertEquals;

class ClearInventoryCommandParsingTest {
    @BeforeAll
    static void bootstrapMinecraft() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void itemPredicateDoesNotConsumeClearMaxCountAsComponentConditions() throws CommandSyntaxException {
        ItemPredicateArgument argument = ItemPredicateArgument.itemPredicate(commandBuildContext());

        StringReader reader = new StringReader("rotten_flesh 9");

        assertNotNull(argument.parse(reader));
        assertEquals(" 9", reader.getRemaining());
    }

    private static CommandBuildContext commandBuildContext() {
        return CommandBuildContext.simple(
            RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY),
            FeatureFlags.DEFAULT_FLAGS
        );
    }
}
