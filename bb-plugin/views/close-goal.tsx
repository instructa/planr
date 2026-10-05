// Close is the one human action that asks first: it ends a goal.
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { goalBuilt, type BoardGoal } from "@/lib/board";
import type { BoardActions } from "@/hooks/use-actions";

export function CloseGoalDialog({
  goal,
  hasRemote,
  actions,
  onOpenChange,
}: {
  goal: BoardGoal | null;
  hasRemote: boolean;
  actions: BoardActions;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={goal !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        {goal !== null ? (
          <>
            <DialogHeader>
              <DialogTitle>Close goal {goal.name}?</DialogTitle>
              <DialogDescription>
                {goal.title ? `${goal.title}. ` : ""}
                {goalBuilt(goal)} of {goal.total} tasks built
                {goal.review > 0 ? `, ${goal.review} still waiting for your check` : ""}. Closing sets the
                goal to done and commits it to the planning repository
                {hasRemote ? ", then pushes to origin" : ""}.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" size="sm" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button
                size="sm"
                disabled={actions.isPending(goal.file) || actions.blocked !== null}
                onClick={() => {
                  onOpenChange(false);
                  void actions.apply({ action: { kind: "close-goal", goal: goal.name }, file: goal.file });
                }}
              >
                Close goal
              </Button>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
