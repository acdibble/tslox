// deno-lint-ignore-file no-namespace
import Expr from "./Expr.ts";
import Token from "./Token.ts";

abstract class Stmt {
  abstract accept<T>(visitor: Stmt.Visitor<T>): T;
}

namespace Stmt {
  export interface Visitor<T> {
    visitBlockStmt(stmt: Block): T;
    visitExpressionStmt(stmt: Expression): T;
    visitIfStmt(stmt: If): T;
    visitPrintStmt(stmt: Print): T;
    visitVarStmt(stmt: Var): T;
  }

  export class Block extends Stmt {
    constructor(
      readonly statements: Stmt[],
    ) {
      super();
    }

    accept<T>(visitor: Stmt.Visitor<T>): T {
      return visitor.visitBlockStmt(this);
    }
  }

  export class Expression extends Stmt {
    constructor(
      readonly expression: Expr,
    ) {
      super();
    }

    accept<T>(visitor: Stmt.Visitor<T>): T {
      return visitor.visitExpressionStmt(this);
    }
  }

  export class If extends Stmt {
    constructor(
      readonly condition: Expr,
      readonly thenBranch: Stmt,
      readonly elseBranch: Stmt | null,
    ) {
      super();
    }

    accept<T>(visitor: Stmt.Visitor<T>): T {
      return visitor.visitIfStmt(this);
    }
  }

  export class Print extends Stmt {
    constructor(
      readonly expression: Expr,
    ) {
      super();
    }

    accept<T>(visitor: Stmt.Visitor<T>): T {
      return visitor.visitPrintStmt(this);
    }
  }

  export class Var extends Stmt {
    constructor(
      readonly name: Token,
      readonly initializer: Expr | null,
    ) {
      super();
    }

    accept<T>(visitor: Stmt.Visitor<T>): T {
      return visitor.visitVarStmt(this);
    }
  }
}

export default Stmt;
