use diesel::prelude::*;
use shared::{
    db::{
        models::{self, Bot, Game, Team},
        schema::game_results,
    },
    GameError, GameResult, GameStatus, GameStatusMessage,
};

use crate::rating::get_rating_change;

pub async fn handle_game_result(status: GameStatusMessage) -> Result<(), ()> {
    use shared::db::schema::{bots, games};
    let db_conn = &mut (*shared::db::conn::DB_CONNECTION.get().map_err(|_| ())?);
    let GameStatusMessage { id, result } = status;
    let (defender_score, challenger_score, error_type, error_bot) = match result.clone() {
        Ok(GameStatus::ScoreChanged(defender_score, challenger_score)) => {
            (defender_score, challenger_score, None, None)
        }
        Ok(GameStatus::TestGameSucceeded) => (0, 0, None, None),
        Ok(GameStatus::TestGameFailed) => (0, 0, None, None),
        Err(e) => match e {
            GameError::InternalError => (100, 100, Some("INTERNAL".into()), None),
            GameError::InvalidActionError(which_bot) => match which_bot {
                shared::WhichBot::Defender => (-100, 100, Some("INVALID_ACTION".into()), Some(0)),
                shared::WhichBot::Challenger => (100, -100, Some("INVALID_ACTION".into()), Some(1)),
            },
            GameError::MemoryError(which_bot) => match which_bot {
                shared::WhichBot::Defender => (-100, 100, Some("MEMORY".into()), Some(0)),
                shared::WhichBot::Challenger => (100, -100, Some("MEMORY".into()), Some(1)),
            },
            GameError::RunTimeError(which_bot) => match which_bot {
                shared::WhichBot::Defender => (-100, 100, Some("RUNTIME".into()), Some(0)),
                shared::WhichBot::Challenger => (100, -100, Some("RUNTIME".into()), Some(1)),
            },
            GameError::TimeoutError(which_bot) => match which_bot {
                shared::WhichBot::Defender => (-100, 100, Some("TIMEOUT".into()), Some(0)),
                shared::WhichBot::Challenger => (100, -100, Some("TIMEOUT".into()), Some(1)),
            },
        },
    };
    // calculate the bots ratings
    let score = (defender_score as f32) / (100.0f32);
    log::info!(
        "Score: {}, defender score {}, challenger score {}",
        score,
        defender_score,
        challenger_score
    );
    let game: Game = games::table
        .find(&id)
        .first::<Game>(db_conn)
        .map_err(|_| ())?;
    let (mut defender_rating_change, mut challenger_rating_change) = get_rating_change(
        game.defender_rating,
        score,
        game.challenger_rating,
        1.0 - score,
    );
    match result {
        Ok(GameStatus::ScoreChanged(_, _)) | Err(_) => {
            // Update rating
            let defender: Bot = diesel::update(bots::table.find(game.defender))
                .set(bots::dsl::rating.eq(bots::dsl::rating + defender_rating_change))
                .get_result::<Bot>(db_conn)
                .map_err(|_| ())?;
            let challenger: Bot = diesel::update(bots::table.find(game.challenger))
                .set(bots::dsl::rating.eq(bots::dsl::rating + challenger_rating_change))
                .get_result::<Bot>(db_conn)
                .map_err(|_| ())?;

            // Update team rating
            diesel::update(shared::db::schema::teams::dsl::teams)
                .filter(shared::db::schema::teams::dsl::id.eq(defender.team))
                .set(
                    shared::db::schema::teams::dsl::score
                        .eq((defender.rating + defender_rating_change).round() as i32),
                )
                .execute(db_conn)
                .map_err(|_| ())?;
            diesel::update(shared::db::schema::teams::dsl::teams)
                .filter(shared::db::schema::teams::dsl::id.eq(challenger.team))
                .set(
                    shared::db::schema::teams::dsl::score
                        .eq((challenger.rating + challenger_rating_change).round() as i32),
                )
                .execute(db_conn)
                .map_err(|_| ())?;
        }
        Ok(GameStatus::TestGameSucceeded) => {
            defender_rating_change = 0.0;
            challenger_rating_change = 0.0;
            // set the active bot for the team if they don't have one
            let (bot, team): (Bot, Team) =
                shared::db::schema::bots::dsl::bots
                    .find(id.parse::<i32>().map_err(|_| ())?)
                    .inner_join(shared::db::schema::teams::dsl::teams.on(
                        shared::db::schema::bots::dsl::team.eq(shared::db::schema::teams::dsl::id),
                    ))
                    .first::<(Bot, Team)>(db_conn)
                    .map_err(|_| ())?;
            log::debug!("Bot: {:?}, team: {:?}", bot, team);
            if team.active_bot.is_none() {
                diesel::update(shared::db::schema::teams::dsl::teams)
                    .filter(shared::db::schema::teams::dsl::id.eq(team.id))
                    .set(shared::db::schema::teams::dsl::active_bot.eq(bot.id))
                    .execute(db_conn)
                    .map_err(|_| ())?;
            }
            diesel::update(bots::dsl::bots)
                .filter(bots::dsl::id.eq(id.parse::<i32>().map_err(|_| ())?))
                .set(bots::dsl::build_status.eq(shared::BuildStatus::TestGameSucceeded as i32))
                .execute(db_conn)
                .map_err(|_| ())?;
        }
        Ok(GameStatus::TestGameFailed) => {
            defender_rating_change = 0.0;
            challenger_rating_change = 0.0;
            diesel::update(bots::dsl::bots)
                .filter(bots::dsl::id.eq(id.parse::<i32>().map_err(|_| ())?))
                .set(bots::dsl::build_status.eq(shared::BuildStatus::TestGameFailed as i32))
                .execute(db_conn)
                .map_err(|_| ())?;
        }
    };

    diesel::insert_into(game_results::table)
        .values(models::NewGameResult {
            id,
            challenger_rating_change,
            defender_rating_change,
            defender_score,
            challenger_score,
            error_type,
            error_bot,
        })
        .execute(db_conn)
        .map_err(|_| ())?;
    Ok(())
}
