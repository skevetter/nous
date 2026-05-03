use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Rooms::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Rooms::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(Rooms::Name).text().not_null())
                    .col(ColumnDef::new(Rooms::Purpose).text())
                    .col(ColumnDef::new(Rooms::Metadata).text())
                    .col(
                        ColumnDef::new(Rooms::Archived)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Rooms::CreatedAt)
                            .text()
                            .not_null()
                            .default(Expr::cust("(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))")),
                    )
                    .col(
                        ColumnDef::new(Rooms::UpdatedAt)
                            .text()
                            .not_null()
                            .default(Expr::cust("(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))")),
                    )
                    .to_owned(),
            )
            .await?;

        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_rooms_name_active ON rooms(name) WHERE archived = 0"
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Rooms::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Rooms {
    Table,
    Id,
    Name,
    Purpose,
    Metadata,
    Archived,
    CreatedAt,
    UpdatedAt,
}
