import { SetNumber as SetNumberEvent } from "../generated/Counter/Counter";
import { SetNumber } from "../generated/schema";

export function handleSetNumber(event: SetNumberEvent): void {
  let entity = new SetNumber(event.transaction.hash.concatI32(event.logIndex.toI32()));
  entity.newNumber = event.params.newNumber;
  entity.save();
}
