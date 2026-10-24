// Copyright (C) 2026 Ryan Daum <ryan.daum@gmail.com> This program is free
// software: you can redistribute it and/or modify it under the terms of the GNU
// General Public License as published by the Free Software Foundation, version
// 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

import { CompileContext } from "@moor/schema/generated/moor-common/compile-context";
import { CompileError } from "@moor/schema/generated/moor-common/compile-error";
import { CompileErrorUnion } from "@moor/schema/generated/moor-common/compile-error-union";
import { InvalidAssignment } from "@moor/schema/generated/moor-common/invalid-assignment";
import { ParseError } from "@moor/schema/generated/moor-common/parse-error";
import { UnknownBuiltinFunction } from "@moor/schema/generated/moor-common/unknown-builtin-function";
import { Builder, ByteBuffer } from "flatbuffers";
import { expect, it } from "vitest";
import { describeCompileError, formatCompileError } from "../../../web-sdk/src/compile-errors";

function compileError(build: (b: Builder) => [CompileErrorUnion, number]): CompileError {
    const b = new Builder();
    const [type, detail] = build(b);
    b.finish(CompileError.createCompileError(b, type, detail));
    return CompileError.getRootAsCompileError(new ByteBuffer(b.asUint8Array()));
}

it("describes errors that are not parse errors with their position and subject", () => {
    const unknown = compileError(b => {
        const context = CompileContext.createCompileContext(b, 3n, 9n);
        const name = b.createString("frobnicate");
        UnknownBuiltinFunction.startUnknownBuiltinFunction(b);
        UnknownBuiltinFunction.addContext(b, context);
        UnknownBuiltinFunction.addName(b, name);
        return [CompileErrorUnion.UnknownBuiltinFunction, UnknownBuiltinFunction.endUnknownBuiltinFunction(b)];
    });
    expect(describeCompileError(unknown)).toEqual({
        message: "Unknown built-in function: frobnicate",
        line: 3,
        column: 9,
    });
    const assignment = compileError(b => {
        const context = CompileContext.createCompileContext(b, 1n, 1n);
        InvalidAssignment.startInvalidAssignment(b);
        InvalidAssignment.addContext(b, context);
        return [CompileErrorUnion.InvalidAssignment, InvalidAssignment.endInvalidAssignment(b)];
    });
    expect(formatCompileError(assignment)).toBe("Line 1, column 1: Invalid l-value for assignment");
});

it("keeps a parse error's own message and position", () => {
    const parse = compileError(b => {
        const position = CompileContext.createCompileContext(b, 2n, 5n);
        const context = b.createString("eval");
        const message = b.createString("expected ';'");
        ParseError.startParseError(b);
        ParseError.addErrorPosition(b, position);
        ParseError.addContext(b, context);
        ParseError.addMessage(b, message);
        return [CompileErrorUnion.ParseError, ParseError.endParseError(b)];
    });
    expect(formatCompileError(parse)).toBe("Line 2, column 5: expected ';'");
});
