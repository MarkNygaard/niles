import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { MeCard } from "@/components/MeCard";
import { api } from "@/lib/api";

/**
 * Your own page, beside Settings on the Me tab rather than inside it.
 *
 * Settings is the house — lights, rooms, integrations — and most of the
 * household never needs it. Everyone has a birthday and a way they like
 * to be spoken to, so that sits where a phone app keeps your profile.
 */
export function MyProfile() {
  const queryClient = useQueryClient();
  // Answers 401 to the API token, which is nobody in particular.
  const me = useQuery({ queryKey: ["me"], queryFn: api.me, retry: false });
  // The dashboard's key, so pairing here takes its offer away there.
  const phone = useQuery({
    queryKey: ["presence-device"],
    queryFn: api.deviceStatus,
    retry: false,
  });
  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["me"] });
    queryClient.invalidateQueries({ queryKey: ["presence-device"] });
    queryClient.invalidateQueries({ queryKey: ["voices"] });
  };
  const update = useMutation({ mutationFn: api.updateMe, onSuccess: refresh });
  const pair = useMutation({ mutationFn: api.pairDevice, onSuccess: refresh });
  const unpair = useMutation({ mutationFn: api.unpairPhone, onSuccess: refresh });

  if (me.isError) {
    return (
      <p className="text-muted-foreground text-sm">
        This page belongs to whoever is signed in, and nobody is.
      </p>
    );
  }

  return (
    <MeCard
      me={me.data}
      device={phone.data}
      saving={update.isPending || pair.isPending || unpair.isPending}
      error={update.error?.message ?? unpair.error?.message ?? pair.error?.message}
      onSave={(change) => update.mutate(change)}
      onPair={() => pair.mutate()}
      onUnpair={() => unpair.mutate()}
    />
  );
}
