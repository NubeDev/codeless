// Error variants are wire-stable — mirror of
// `codeless/crates/codeless-rpc/src/error.rs`. Renaming a kind is a
// breaking change.

export type RpcErrorKind =
  | "not_found"
  | "invalid_argument"
  | "conflict"
  | "internal";

export class RpcError extends Error {
  readonly kind: RpcErrorKind;

  constructor(kind: RpcErrorKind, message: string) {
    super(`${kind}: ${message}`);
    this.name = "RpcError";
    this.kind = kind;
  }

  static fromHttpStatus(status: number, message: string): RpcError {
    switch (status) {
      case 404:
        return new RpcError("not_found", message);
      case 400:
        return new RpcError("invalid_argument", message);
      case 409:
        return new RpcError("conflict", message);
      default:
        return new RpcError("internal", message);
    }
  }
}
