// Copyright (C) 2026 Ryan Daum <ryan.daum@gmail.com> This program is free
// software: you can redistribute it and/or modify it under the terms of the GNU
// Affero General Public License as published by the Free Software Foundation,
// version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more
// details.
//
// You should have received a copy of the GNU Affero General Public License along
// with this program. If not, see <https://www.gnu.org/licenses/>.

import type { Tool } from "@modelcontextprotocol/server";

export function withCharacter(schema: Record<string, unknown>): Tool["inputSchema"] {
    const source = schema as {
        properties?: Tool["inputSchema"]["properties"];
        required?: string[];
    };
    const properties = { ...(source.properties ?? {}) };
    properties.moo = {
        type: "string",
        description: "Configured moo id. Defaults to config.defaultMoo.",
    };
    properties.character = {
        type: "string",
        description: "Configured player id inside the selected moo.",
    };
    properties.wizard = {
        type: "boolean",
        description: "Use a player marked isWizard=true in the selected moo.",
        default: false,
    };
    return {
        additionalProperties: false,
        ...source,
        type: "object",
        properties,
    };
}

const object = {
    type: "string",
    description: "Object: #123, a uuid object like #000123-ABCDEF0123, $name for a system object, or me / here.",
} as const;

export const staticTools: Tool[] = [
    {
        name: "moo_eval",
        description: "Run MOO code as the character and return its value. A single expression is returned; "
            + "code with statements runs as written and returns what it returns, so end it with a return.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                expression: {
                    type: "string",
                    description: "An expression such as `player.name`, or statements such as `x = 1; return x + 1;`.",
                },
            },
            required: ["expression"],
        }),
    },
    {
        name: "moo_command",
        description: "Type a command as the character, as a player would, and return the narrative output.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                command: { type: "string", description: "The command line, such as `look` or `go north`." },
                include_metadata: {
                    type: "boolean",
                    description: "Include parsed websocket event metadata JSON in the response.",
                    default: false,
                },
            },
            required: ["command"],
        }),
    },
    {
        name: "moo_function_help",
        description: "Get documentation for a MOO builtin function.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                function_name: { type: "string", description: "The builtin's name, such as `notify`." },
            },
            required: ["function_name"],
        }),
    },
    {
        name: "moo_invoke_verb",
        description: "Call a verb on an object with the given arguments and return its value.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                object,
                verb: { type: "string", description: "The verb to call." },
                args: { type: "array", items: {}, description: "Arguments, as JSON values. Defaults to none." },
            },
            required: ["object", "verb"],
        }),
    },
    {
        name: "moo_list_objects",
        description: "List objects visible to the selected character.",
        inputSchema: withCharacter({
            type: "object",
            properties: {},
        }),
    },
    {
        name: "moo_resolve",
        description: "Resolve an object reference to the object it names.",
        inputSchema: withCharacter({
            type: "object",
            properties: { object },
            required: ["object"],
        }),
    },
    {
        name: "moo_list_verbs",
        description: "List the verbs defined on an object.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                object,
                inherited: { type: "boolean", description: "Include verbs inherited from ancestors." },
            },
            required: ["object"],
        }),
    },
    {
        name: "moo_get_verb",
        description: "Get a verb's code and metadata.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                object,
                name: { type: "string", description: "The verb's name." },
            },
            required: ["object", "name"],
        }),
    },
    {
        name: "moo_program_verb",
        description: "Replace an existing verb's code. Compile errors are reported with their line and column.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                object,
                name: { type: "string", description: "The verb's name." },
                code: { type: "string", description: "The verb's complete new code." },
            },
            required: ["object", "name", "code"],
        }),
    },
    {
        name: "moo_list_properties",
        description: "List the properties defined on an object.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                object,
                inherited: { type: "boolean", description: "Include properties inherited from ancestors." },
            },
            required: ["object"],
        }),
    },
    {
        name: "moo_get_property",
        description: "Get one property's value, including built-in properties such as name and owner.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                object,
                name: { type: "string", description: "The property's name." },
            },
            required: ["object", "name"],
        }),
    },
    {
        name: "moo_set_property",
        description: "Set a property's value, written as a MOO literal.",
        inputSchema: withCharacter({
            type: "object",
            properties: {
                object,
                name: { type: "string", description: "The property's name." },
                valueLiteral: {
                    type: "string",
                    description: "The new value as a MOO literal, such as `{1, \"two\"}`.",
                },
            },
            required: ["object", "name", "valueLiteral"],
        }),
    },
    {
        name: "moo_refresh_dynamic_tools",
        description: "Reload the world's tools from #0:external_agent_tools(), for example after an update.",
        inputSchema: withCharacter({
            type: "object",
            properties: {},
        }),
    },
];
