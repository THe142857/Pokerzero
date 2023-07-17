use itertools::Itertools;
use rand::{thread_rng, Rng};
use shared::{Bot, GameError, GameResult, WhichBot};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    fs,
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdout, Command},
    time::Instant,
    try_join,
};

use crate::poker::game::GameState;

pub mod sandbox;
pub async fn download_and_run<T: Into<String>, U: Into<String>, V: Into<PathBuf>>(
    bot: U,
    bot_path: V,
    bot_bucket: T,
    s3_client: &aws_sdk_s3::Client,
) -> Result<tokio::process::Child, GameError> {
    let bot_path: PathBuf = bot_path.into();
    shared::s3::download_file(
        &bot.into(),
        &bot_path.join("bot.zip"),
        &bot_bucket.into(),
        &s3_client,
    )
    .await?;

    log::debug!("Bot downloaded");
    Command::new("unzip")
        .arg(&bot_path.join("bot.zip"))
        .current_dir(&bot_path)
        .spawn()?
        .wait()
        .await?;
    log::debug!("Bot unzipped to {:?}", bot_path);

    let bot_json: Bot = async {
        let json = fs::read_to_string(&bot_path.join("bot/bot.json")).await?;
        if let Ok(bot) = serde_json::from_str::<Bot>(&json) {
            return Ok(bot);
        }
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Unable to parse bot.json",
        ))
    }
    .await?;
    log::debug!("Read json");

    let log_file = Stdio::from(std::fs::File::create(bot_path.join("logs")).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("Failed to create log file: {}", e),
        )
    })?);
    Command::new("sh")
        .arg("-c")
        .arg(bot_json.run)
        .current_dir(&bot_path.join("bot"))
        .stderr(log_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| {
            log::error!("Error running bot: {}", e);
            GameError::InternalError
        })
}

pub async fn run_game(
    bot_a: &String,
    bot_b: &String,
    s3_client: &aws_sdk_s3::Client,
    task_id: &String,
    rounds: usize,
    game_path: &mut PathBuf,
) -> GameResult {
    // create tmp directory
    // doesn't have the same id as the task
    let game_id = format!("{:x}", rand::thread_rng().gen::<u32>());
    let tmp_dir = Path::new("/tmp").join(&game_id);
    *game_path = tmp_dir.clone();
    log::debug!("Playing {} against {}", bot_a, bot_b);
    log::info!("Running game {} with local id {}", task_id, game_id);
    let bot_bucket = std::env::var("COMPILED_BOT_S3_BUCKET").map_err(|e| {
        log::error!("Error getting COMPILED_BOT_S3_BUCKET: {}", e);
        GameError::InternalError
    })?;
    log::debug!("Bot bucket: {}", bot_bucket);

    // download bots from s3
    log::debug!("Making bot directories");
    let bot_a_path = tmp_dir.join("bot_a");
    fs::create_dir_all(&bot_a_path).await.map_err(|e| {
        log::error!("Error creating bot_a directory: {}", e);
        shared::GameError::InternalError
    })?;
    let bot_b_path = tmp_dir.join("bot_b");
    fs::create_dir_all(&bot_b_path).await.map_err(|e| {
        log::error!("Error creating bot_b directory: {}", e);
        shared::GameError::InternalError
    })?;
    log::debug!("Downloading bots from aws");
    let (bot_a, bot_b) = try_join!(
        download_and_run(bot_a, bot_a_path, &bot_bucket, s3_client),
        download_and_run(bot_b, bot_b_path, &bot_bucket, s3_client)
    )?;

    // run game
    let mut game = Game::new(
        bot_a,
        bot_b,
        game_id,
        Duration::from_secs(1),
        tokio::fs::File::create(tmp_dir.join("logs")).await?,
    );

    game.play(rounds).await
}

pub struct Game {
    bot_a: tokio::process::Child,
    bot_b: tokio::process::Child,
    stacks: [u32; 2],
    initial_stacks: [u32; 2],
    button: usize,
    id: String,
    timeout: Duration,
    logs: tokio::fs::File,
    start_time: Instant,
}
impl Game {
    pub fn new(
        bot_a: tokio::process::Child,
        bot_b: tokio::process::Child,
        id: String,
        timeout: Duration,
        logs: tokio::fs::File,
    ) -> Self {
        Self {
            bot_a,
            bot_b,
            stacks: [50, 50],
            initial_stacks: [50, 50],
            button: 0,
            timeout,
            id,
            logs,
            start_time: Instant::now(),
        }
    }

    async fn write_bot<T: Into<String>>(
        &mut self,
        which_bot: WhichBot,
        message: T,
    ) -> Result<(), GameError> {
        let message: String = message.into();
        self.write_log(format!("{} < {}", which_bot, message.clone()))
            .await?;
        let bot = match which_bot {
            WhichBot::BotA => &mut self.bot_a,
            WhichBot::BotB => &mut self.bot_b,
        };
        if let Some(ref mut stdin) = bot.stdin {
            stdin
                .write_all(format!("{}\n", message).as_bytes())
                .await
                .map_err(|_| {
                    log::error!("Error writing to bot");
                    GameError::RunTimeError(which_bot)
                })?;

            stdin.flush().await.map_err(|_| {
                log::error!("Error writing to bot");
                GameError::RunTimeError(which_bot)
            })?;
            Ok(())
        } else {
            // TODO: determine cause of close
            self.write_log(format!("System > Ending because {} lost stdin", which_bot))
                .await?;
            Err(GameError::RunTimeError(which_bot))
        }
    }

    async fn print_position(&mut self, which_bot: WhichBot) -> Result<(), GameError> {
        let position = format!(
            "P {}",
            match which_bot {
                WhichBot::BotA => self.button,
                WhichBot::BotB => (self.button + 1) % 2,
            }
        );
        self.write_bot(which_bot, position).await?;
        Ok(())
    }

    async fn print_round_end(&mut self, which_bot: WhichBot) -> Result<(), shared::GameError> {
        self.write_bot(which_bot, "E").await?;
        Ok(())
    }

    async fn print_cards(
        &mut self,
        which_bot: WhichBot,
        state: &GameState,
    ) -> Result<(), shared::GameError> {
        let cards = [
            state.player_states[match which_bot {
                WhichBot::BotA => self.button,
                WhichBot::BotB => 1 - self.button,
            }]
            .hole_cards
            .clone(),
            state.community_cards.clone(),
        ]
        .concat()
        .iter()
        .map(|card| format!("{}", card))
        .join(" ");
        self.write_bot(which_bot, format!("C {}", cards)).await?;

        Ok(())
    }

    async fn write_log<S: Into<String>>(&mut self, msg: S) -> Result<(), shared::GameError> {
        self.logs
            .write_all(
                format!(
                    "{}ms {}\n",
                    tokio::time::Instant::now()
                        .duration_since(self.start_time)
                        .as_millis(),
                    msg.into()
                )
                .as_bytes(),
            )
            .await?;
        Ok(())
    }

    async fn play_round(
        &mut self,
        bot_a_reader: &mut BufReader<ChildStdout>,
        bot_b_reader: &mut BufReader<ChildStdout>,
    ) -> Result<(), shared::GameError> {
        let mut rng = thread_rng();
        let mut state = crate::poker::game::GameState::new(
            if self.button == 1 {
                [self.stacks[1], self.stacks[0]]
            } else {
                [self.stacks[0], self.stacks[1]]
            },
            GameState::get_shuffled_deck(&mut rng),
        );

        log::debug!("Game state: {:?}. ", state);

        let mut round = None;

        self.print_position(WhichBot::BotA).await.map_err(|_| {
            log::info!("Failed to print position to bot A.");
            GameError::RunTimeError(WhichBot::BotA)
        })?;

        self.print_position(WhichBot::BotB).await.map_err(|_| {
            log::info!("Failed to print position to bot B.");
            GameError::RunTimeError(WhichBot::BotB)
        })?;

        loop {
            self.stacks = if self.button == 1 {
                [state.get_stack(true), state.get_stack(false)]
            } else {
                [state.get_stack(false), state.get_stack(true)]
            };

            if state.round_over() {
                log::debug!("Round ended.");
                self.print_round_end(WhichBot::BotA).await.map_err(|_| {
                    log::info!("Failed to print round end to bot A.");
                    GameError::RunTimeError(WhichBot::BotA)
                })?;

                self.print_round_end(WhichBot::BotB).await.map_err(|_| {
                    log::info!("Failed to print round end to bot B.");
                    GameError::RunTimeError(WhichBot::BotB)
                })?;
                break;
            }
            // Print community cards to both bots
            if round != Some(state.round) {
                log::debug!("Printing community cards.");
                round = Some(state.round);
                self.print_cards(WhichBot::BotA, &state)
                    .await
                    .map_err(|_| {
                        log::info!("Failed to print community cards to bot A.");
                        GameError::RunTimeError(WhichBot::BotA)
                    })?;
                self.print_cards(WhichBot::BotB, &state)
                    .await
                    .map_err(|_| {
                        log::info!("Failed to print community cards to bot B.");
                        GameError::RunTimeError(WhichBot::BotB)
                    })?;
            }
            // Assume state.whose_turn() is not None
            let whose_turn: WhichBot =
                if state.whose_turn().ok_or(GameError::InternalError)? == self.button {
                    WhichBot::BotA
                } else {
                    WhichBot::BotB
                };

            let target_reader = match whose_turn {
                WhichBot::BotA => &mut *bot_a_reader,
                WhichBot::BotB => &mut *bot_b_reader,
            };

            // write current game state to the bots stream
            log::debug!("Writing current state.");
            let status = format!(
                "S {} {} {} {} {}",
                state.target_push,
                state.player_states[0].pushed,
                state.player_states[1].pushed,
                state.player_states[0].stack,
                state.player_states[1].stack,
            );
            self.write_bot(whose_turn, status).await.map_err(|_| {
                log::info!("Failed to write current state to bot {:?}.", whose_turn);
                GameError::RunTimeError(whose_turn)
            })?;
            log::debug!("Reading action from {:?}.", whose_turn);
            let mut line: String = Default::default();
            tokio::time::timeout(self.timeout, target_reader.read_line(&mut line))
                .await
                .map_err(|_| shared::GameError::TimeoutError(whose_turn))?
                .map_err(|_| shared::GameError::RunTimeError(whose_turn))?;
            self.write_log(format!("{} > {}", whose_turn, line.trim()))
                .await?;
            log::debug!("Reading action from {:?}.", line);
            state = state
                .post_action(
                    parse_action(line.trim())
                        .map_err(|_| shared::GameError::InvalidActionError(whose_turn.clone()))?,
                )
                .map_err(|_| shared::GameError::InvalidActionError(whose_turn.clone()))?;
        }

        Ok(())
    }
    /// Play a game of poker, returning a [shared::GameResult]
    pub async fn play(&mut self, rounds: usize) -> shared::GameResult {
        log::debug!("Playing game {} with {} rounds", self.id, rounds);
        let mut bot_a_reader = BufReader::new(
            self.bot_a
                .stdout
                .take()
                .ok_or(GameError::RunTimeError(WhichBot::BotA))?,
        );
        let mut bot_b_reader = BufReader::new(
            self.bot_b
                .stdout
                .take()
                .ok_or(GameError::RunTimeError(WhichBot::BotB))?,
        );

        log::info!("Clients connected for {}", self.id);
        for i in 0..rounds {
            if self.stacks[0] == 0 || self.stacks[1] == 0 {
                self.write_log(format!("System > Ending because a bot has an empty stack"))
                    .await?;
                break;
            }
            self.write_log(format!("System > round {}/{}", i + 1, rounds))
                .await?;
            log::debug!("Playing round. Current stacks: {:?}.", self.stacks);
            if let Err(e) = self.play_round(&mut bot_a_reader, &mut bot_b_reader).await {
                self.write_log(format!("System > {:?}", e)).await?;
                Err(e)?;
            }
            self.button = 1 - self.button;
        }
        return Ok(shared::GameStatus::ScoreChanged(
            i32::try_from(self.stacks[0]).unwrap() - i32::try_from(self.initial_stacks[0]).unwrap(),
        ));
    }
}

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

impl Drop for Game {
    fn drop(&mut self) {
        if let Some(id) = self.bot_a.id() {
            unsafe {
                kill(id.try_into().unwrap(), 9);
            }
        }
        if let Some(id) = self.bot_b.id() {
            unsafe {
                kill(id.try_into().unwrap(), 9);
            }
        }
    }
}

fn parse_action<T: AsRef<str>>(
    line: T,
) -> Result<crate::poker::game::Action, shared::GameActionError> {
    let line = line.as_ref();
    Ok(match line.as_ref() {
        "X" => crate::poker::game::Action::Check,
        "F" => crate::poker::game::Action::Fold,
        "C" => crate::poker::game::Action::Call,
        _ => {
            if line.chars().nth(0) != Some('R') {
                Err(shared::GameActionError::CouldNotParse)?;
            }
            let amount = line[1..]
                .parse::<u32>()
                .map_err(|_| shared::GameActionError::CouldNotParse)?;
            crate::poker::game::Action::Raise { amt: amount }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::parse_action;
    #[test]
    fn parse_action_check() {
        assert_eq!(
            parse_action(&"X".to_owned()).unwrap(),
            crate::poker::game::Action::Check
        );
    }

    #[test]
    fn parse_action_fold() {
        assert_eq!(
            parse_action(&"F".to_owned()).unwrap(),
            crate::poker::game::Action::Fold
        );
    }

    #[test]
    fn parse_action_call() {
        assert_eq!(
            parse_action(&"C".to_owned()).unwrap(),
            crate::poker::game::Action::Call
        );
    }

    #[test]
    fn parse_action_raise() {
        assert_eq!(
            parse_action(&"R1234".to_owned()).unwrap(),
            crate::poker::game::Action::Raise { amt: 1234 }
        );
    }

    #[test]
    fn parse_action_raise_invalid() {
        assert!(parse_action(&"R".to_owned()).is_err());
    }

    #[test]
    fn parse_action_raise_invalid2() {
        assert!(parse_action(&"R1234a".to_owned()).is_err());
    }

    #[test]
    fn parse_action_raise_invalid3() {
        assert!(parse_action(&"R-1234".to_owned()).is_err());
    }

    #[test]
    fn parse_action_raise_invalid4() {
        assert!(parse_action(&"R-1".to_owned()).is_err());
    }

    #[test]
    fn parse_action_raise_invalid5() {
        assert!(parse_action(&"R1234.0".to_owned()).is_err());
    }

    #[test]
    fn parse_action_raise_invalid6() {
        assert!(parse_action(&"B".to_owned()).is_err());
    }
}
