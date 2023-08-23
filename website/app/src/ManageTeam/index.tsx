import React from "react";
import { useUser, useTeam } from "../state";
import CreateTeam from "./CreateTeam";
import Login from "../Login";
import Box from "@mui/system/Box";
import { Container } from "@mui/system";
import { team_member_table_row } from "./styles.module.css";

import { TeamBar } from "./TeamBar";
import BotTable from "./BotTable";
import { BotUpload } from "./BotUpload";
import { GameTable } from "../components/Tables/GameTable";
import Sheet from "@mui/joy/Sheet";
import Stack from "@mui/joy/Stack";

function NoTeam() {
  return (
    <Box
      sx={{
        width: "100%",
        flexGrow: 1,
        padding: "20px",
      }}
    >
      <Container>There is no team at this URL.</Container>
    </Box>
  );
}

export function DisplayTeam({
  readonly,
  teamId,
}: {
  readonly: boolean;
  teamId: string | null;
}) {
  const team = useTeam(teamId)[0];
  console.log(team);
  if (!team) return <NoTeam />;
  return (
    <>
      <TeamBar readonly={readonly} teamId={teamId} />
      <Box
        sx={{
          flexGrow: 1,
        }}
      >
        <Stack gap={2}>
          <Sheet sx={{ p: 4 }}>
            <h2>Bots</h2>
            {!readonly && <BotUpload />}

            <BotTable readonly={readonly} teamId={teamId} />
          </Sheet>
          <Sheet sx={{ p: 4, mb: 4 }}>
            <h2>Games</h2>
            <GameTable teamId={teamId} />
          </Sheet>
        </Stack>
      </Box>
    </>
  );
}

export default function ManageTeam({
  teamId,
  readonly,
}: {
  teamId: string | null;
  readonly: boolean;
}) {
  const [team, fetchTeam] = useTeam(teamId);
  const [user, fetchUser] = useUser();
  if (readonly || (team && user)) {
    return <DisplayTeam readonly={readonly} teamId={teamId} />;
  } else if (user) {
    return <CreateTeam />;
  } else {
    return <Login />;
  }
}
